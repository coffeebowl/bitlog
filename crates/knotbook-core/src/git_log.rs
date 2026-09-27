//! Reading the commits of a project's local Git repository. Knotbook only
//! reads them and keeps nothing, so rebases and rewritten commits do not
//! matter.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use git2::{ErrorCode, Repository, Sort};
use thiserror::Error;

/// A commit as `git log` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The full hash in hex.
    pub id: String,
    pub author: String,
    /// When the author made it, in the author's time zone.
    pub time: DateTime<FixedOffset>,
    /// The first line of the message.
    pub summary: String,
}

impl Commit {
    /// The hash shortened to seven digits, as Git usually shows it.
    pub fn short_id(&self) -> &str {
        &self.id[..7]
    }
}

/// A repository whose log cannot be read.
#[derive(Debug, Error)]
pub enum GitLogError {
    #[error("the repository folder {} does not exist", .0.display())]
    Missing(PathBuf),
    #[error("{} is not a Git repository", .0.display())]
    NotARepository(PathBuf),
    #[error("cannot read the Git log of {}: {message}", path.display())]
    Git { path: PathBuf, message: String },
}

/// The commits of the current branch of the repository in the folder
/// `repo`, newest first, leaving out the first `skip` and taking at most
/// `limit`. A branch without commits yet has none.
///
/// It reads from disk and may take a while in large repositories, so
/// programs with a user interface call it outside the main thread.
pub fn git_log(repo: &Path, skip: usize, limit: usize) -> Result<Vec<Commit>, GitLogError> {
    let git_error = |err: git2::Error| GitLogError::Git {
        path: repo.to_owned(),
        message: err.message().to_owned(),
    };
    if !repo.is_dir() {
        return Err(GitLogError::Missing(repo.to_owned()));
    }
    let repository = Repository::open(repo).map_err(|err| match err.code() {
        ErrorCode::NotFound => GitLogError::NotARepository(repo.to_owned()),
        _ => git_error(err),
    })?;
    let head = match repository.head() {
        // A branch without commits yet.
        Err(err) if err.code() == ErrorCode::UnbornBranch => return Ok(Vec::new()),
        result => result
            .and_then(|head| head.peel_to_commit())
            .map_err(git_error)?,
    };
    let mut walk = repository.revwalk().map_err(git_error)?;
    walk.set_sorting(Sort::TIME).map_err(git_error)?;
    walk.push(head.id()).map_err(git_error)?;
    walk.skip(skip)
        .take(limit)
        .map(|id| {
            let commit = repository.find_commit(id?)?;
            let author = commit.author();
            let when = author.when();
            // Broken commits with impossible times exist; show them in UTC
            // or at 1970 rather than failing.
            let offset = FixedOffset::east_opt(when.offset_minutes() * 60)
                .unwrap_or(FixedOffset::east_opt(0).expect("UTC is valid"));
            let time = DateTime::from_timestamp(when.seconds(), 0)
                .unwrap_or_default()
                .with_timezone(&offset);
            Ok(Commit {
                id: commit.id().to_string(),
                author: String::from_utf8_lossy(author.name_bytes()).into_owned(),
                time,
                summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                    .into_owned(),
            })
        })
        .collect::<Result<_, git2::Error>>()
        .map_err(git_error)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use git2::{Signature, Time};

    use super::*;
    use crate::file::TempDir;

    /// A repository with the commits `summaries`, made an hour apart by
    /// Ada, starting at 2026-09-21 09:00 in UTC+2.
    fn repository(dir: &TempDir, summaries: &[&str]) -> Repository {
        let repository = Repository::init(&dir.0).unwrap();
        {
            let id = repository.index().unwrap().write_tree().unwrap();
            let tree = repository.find_tree(id).unwrap();
            let mut parent = None;
            for (hour, summary) in (0..).zip(summaries) {
                let time = Time::new(1_789_974_000 + hour * 3600, 120);
                let author = Signature::new("Ada", "ada@example.org", &time).unwrap();
                let parents: Vec<&git2::Commit> = parent.iter().collect();
                let message = format!("{summary}\n\nDetails.");
                let id = repository
                    .commit(Some("HEAD"), &author, &author, &message, &tree, &parents)
                    .unwrap();
                parent = Some(repository.find_commit(id).unwrap());
            }
        }
        repository
    }

    fn summaries(commits: &[Commit]) -> Vec<&str> {
        commits.iter().map(|c| c.summary.as_str()).collect()
    }

    #[test]
    fn newest_first_in_pages() {
        let dir = TempDir::new();
        repository(&dir, &["First", "Second", "Third", "Fourth"]);
        let all = git_log(&dir.0, 0, 10).unwrap();
        assert_eq!(summaries(&all), ["Fourth", "Third", "Second", "First"]);
        assert_eq!(
            summaries(&git_log(&dir.0, 1, 2).unwrap()),
            ["Third", "Second"]
        );
        assert!(git_log(&dir.0, 4, 10).unwrap().is_empty());

        let first = &all[3];
        assert_eq!(first.author, "Ada");
        assert_eq!(first.time.to_rfc3339(), "2026-09-21T09:00:00+02:00");
        assert_eq!(first.id.len(), 40);
        assert_eq!(first.short_id(), &first.id[..7]);
    }

    #[test]
    fn only_the_current_branch() {
        let dir = TempDir::new();
        let repository = repository(&dir, &["First", "Second"]);
        let first = repository.head().unwrap().peel_to_commit().unwrap();
        let first = first.parent(0).unwrap();
        repository.branch("old", &first, false).unwrap();
        repository.set_head("refs/heads/old").unwrap();
        assert_eq!(summaries(&git_log(&dir.0, 0, 10).unwrap()), ["First"]);
        // A detached HEAD is read as well.
        repository.set_head_detached(first.id()).unwrap();
        assert_eq!(summaries(&git_log(&dir.0, 0, 10).unwrap()), ["First"]);
    }

    #[test]
    fn no_commits_yet() {
        let dir = TempDir::new();
        repository(&dir, &[]);
        assert!(git_log(&dir.0, 0, 10).unwrap().is_empty());
    }

    #[test]
    fn missing_or_no_repository() {
        let dir = TempDir::new();
        let missing = dir.0.join("gone");
        assert_eq!(
            git_log(&missing, 0, 10).unwrap_err().to_string(),
            format!("the repository folder {} does not exist", missing.display())
        );
        fs::create_dir_all(&missing).unwrap();
        assert_eq!(
            git_log(&missing, 0, 10).unwrap_err().to_string(),
            format!("{} is not a Git repository", missing.display())
        );
    }
}
