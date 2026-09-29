//! Reading the commits of a project's local Git repository. Knotbook only
//! reads them and keeps nothing, so rebases and rewritten commits do not
//! matter.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use git2::{
    BranchType, Delta, DiffFindOptions, ErrorCode, Oid, Patch, ReferenceType, Repository,
    Signature, Sort, Status, StatusOptions,
};
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

/// A branch of a repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Branch {
    /// A local branch, such as `main`.
    Local(String),
    /// A remote-tracking branch, such as `origin/main`.
    Remote(String),
}

impl Branch {
    /// The name as Git shows it, such as `main` or `origin/main`.
    pub fn name(&self) -> &str {
        match self {
            Branch::Local(name) | Branch::Remote(name) => name,
        }
    }

    fn reference(&self) -> String {
        match self {
            Branch::Local(name) => format!("refs/heads/{name}"),
            Branch::Remote(name) => format!("refs/remotes/{name}"),
        }
    }
}

/// The branches of a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branches {
    /// The local branches, sorted, with the one checked out even before
    /// its first commit, then the remote-tracking ones, sorted.
    pub all: Vec<Branch>,
    /// The local branch checked out, `None` if HEAD is detached.
    pub current: Option<String>,
}

/// Where a local branch stands against the branch it tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// The branch tracked, such as `origin/main`.
    pub name: String,
    /// The commits only on the local branch, not pushed yet.
    pub ahead: usize,
    /// The commits only on the branch tracked, not pulled yet.
    pub behind: usize,
}

/// The files of a working tree that differ from the commit checked out,
/// staged or not, as `git status` lists them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Uncommitted {
    /// Files changed, removed or renamed.
    pub changed: usize,
    /// Files Git does not know yet, or only from the index. A new folder
    /// counts once.
    pub new: usize,
}

/// A commit with what it changed, as `git show --stat` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub commit: Commit,
    pub author_email: String,
    /// The whole message, without trailing blank lines.
    pub message: String,
    /// Who committed it and when, if someone else or at another time than
    /// the author, as after a rebase.
    pub committer: Option<(String, DateTime<FixedOffset>)>,
    /// The full hashes of the parents, the first first.
    pub parents: Vec<String>,
    /// The files changed against the first parent, in the order of their
    /// paths.
    pub files: Vec<ChangedFile>,
}

/// A file a commit changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    /// The path before, if it was renamed or copied.
    pub old_path: Option<String>,
    pub change: FileChange,
    /// The lines added and removed, unless it is a binary file.
    pub lines: Option<(usize, usize)>,
}

/// How a commit changed a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    /// Became a link or submodule, or stopped being one.
    TypeChanged,
}

/// The commits of the branch `branch` of the repository in the folder
/// `repo`, or with `None` of the one checked out, newest first,
/// leaving out the first `skip` and taking at most `limit`. A branch
/// without commits yet has none.
///
/// It reads from disk and may take a while in large repositories, so
/// programs with a user interface call it outside the main thread.
pub fn git_log(
    repo: &Path,
    branch: Option<&Branch>,
    skip: usize,
    limit: usize,
) -> Result<Vec<Commit>, GitLogError> {
    let repository = open(repo)?;
    let head = match branch {
        Some(branch) => repository.find_reference(&branch.reference()),
        None => repository.head(),
    };
    let head = match head {
        // A branch without commits yet.
        Err(err) if err.code() == ErrorCode::UnbornBranch => return Ok(Vec::new()),
        result => result
            .and_then(|head| head.peel_to_commit())
            .map_err(git_error(repo))?,
    };
    let mut walk = repository.revwalk().map_err(git_error(repo))?;
    walk.set_sorting(Sort::TIME).map_err(git_error(repo))?;
    walk.push(head.id()).map_err(git_error(repo))?;
    walk.skip(skip)
        .take(limit)
        .map(|id| Ok(commit_of(&repository.find_commit(id?)?)))
        .collect::<Result<_, git2::Error>>()
        .map_err(git_error(repo))
}

/// The local and remote-tracking branches of the repository in the
/// folder `repo`.
pub fn git_branches(repo: &Path) -> Result<Branches, GitLogError> {
    let repository = open(repo)?;
    branches(&repository).map_err(git_error(repo))
}

fn branches(repository: &Repository) -> Result<Branches, git2::Error> {
    let names = |kind| {
        let mut names = Vec::new();
        for branch in repository.branches(Some(kind))? {
            let (branch, _) = branch?;
            // Such as `origin/HEAD`, which only points to another.
            if branch.get().kind() == Some(ReferenceType::Symbolic) {
                continue;
            }
            names.push(String::from_utf8_lossy(branch.name_bytes()?).into_owned());
        }
        Ok::<_, git2::Error>(names)
    };
    let mut local = names(BranchType::Local)?;
    // HEAD names the branch even before its first commit, when there is
    // no branch yet.
    let head = repository.find_reference("HEAD")?;
    let current = head
        .symbolic_target_bytes()
        .and_then(|target| target.strip_prefix(b"refs/heads/"))
        .map(|name| String::from_utf8_lossy(name).into_owned());
    if let Some(current) = &current
        && !local.contains(current)
    {
        local.push(current.clone());
    }
    local.sort();
    let mut remote = names(BranchType::Remote)?;
    remote.sort();
    let all = (local.into_iter().map(Branch::Local))
        .chain(remote.into_iter().map(Branch::Remote))
        .collect();
    Ok(Branches { all, current })
}

/// Where the local branch `branch` of the repository in the folder `repo`
/// stands against the branch it tracks, `None` if it tracks none or has
/// no commits yet.
///
/// It compares the commits of both, so programs with a user interface
/// call it outside the main thread.
pub fn git_upstream(repo: &Path, branch: &str) -> Result<Option<Upstream>, GitLogError> {
    let repository = open(repo)?;
    upstream(&repository, branch).map_err(git_error(repo))
}

fn upstream(repository: &Repository, branch: &str) -> Result<Option<Upstream>, git2::Error> {
    let not_found = |err: &git2::Error| err.code() == ErrorCode::NotFound;
    let local = match repository.find_branch(branch, BranchType::Local) {
        Err(err) if not_found(&err) => return Ok(None),
        result => result?,
    };
    // Also when the branch tracked is gone, as after `git fetch --prune`.
    let upstream = match local.upstream() {
        Err(err) if not_found(&err) => return Ok(None),
        result => result?,
    };
    let (Some(ours), Some(theirs)) = (local.get().target(), upstream.get().target()) else {
        return Ok(None);
    };
    let (ahead, behind) = repository.graph_ahead_behind(ours, theirs)?;
    Ok(Some(Upstream {
        name: String::from_utf8_lossy(upstream.name_bytes()?).into_owned(),
        ahead,
        behind,
    }))
}

/// The files of the working tree of the repository in the folder `repo`
/// that differ from the commit checked out. Ignored files do not count.
///
/// It compares every file with the index, so programs with a user
/// interface call it outside the main thread.
pub fn git_uncommitted(repo: &Path) -> Result<Uncommitted, GitLogError> {
    let repository = open(repo)?;
    let mut options = StatusOptions::new();
    options.include_untracked(true).exclude_submodules(true);
    let statuses = repository
        .statuses(Some(&mut options))
        .map_err(git_error(repo))?;
    let mut uncommitted = Uncommitted::default();
    for entry in statuses.iter() {
        let status = entry.status();
        if status.intersects(Status::WT_NEW | Status::INDEX_NEW) {
            uncommitted.new += 1;
        } else if !status.is_empty() && !status.contains(Status::IGNORED) {
            uncommitted.changed += 1;
        }
    }
    Ok(uncommitted)
}

/// The commit `id`, a full hash, of the repository in the folder `repo`,
/// with what it changed.
///
/// It reads from disk and compares the files the commit changed, so
/// programs with a user interface call it outside the main thread.
pub fn git_commit(repo: &Path, id: &str) -> Result<CommitDetails, GitLogError> {
    let repository = open(repo)?;
    details(&repository, id).map_err(git_error(repo))
}

fn details(repository: &Repository, id: &str) -> Result<CommitDetails, git2::Error> {
    let commit = repository.find_commit(Oid::from_str(id)?)?;
    let author = commit.author();
    let committer = commit.committer();
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let committed = (committer.name_bytes() != author.name_bytes()
        || committer.email_bytes() != author.email_bytes()
        || committer.when() != author.when())
    .then(|| (text(committer.name_bytes()), time_of(&committer)));

    // Against the first parent, which for a merge is what it brought into
    // the branch, or against nothing for the first commit.
    let parent_tree = match commit.parents().next() {
        Some(parent) => Some(parent.tree()?),
        None => None,
    };
    let mut diff =
        repository.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit.tree()?), None)?;
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;
    let mut files = Vec::new();
    for (index, delta) in diff.deltas().enumerate() {
        let path = |file: git2::DiffFile| {
            file.path_bytes()
                .map(|path| String::from_utf8_lossy(path).into_owned())
                .unwrap_or_default()
        };
        let change = match delta.status() {
            Delta::Added => FileChange::Added,
            Delta::Deleted => FileChange::Deleted,
            Delta::Renamed => FileChange::Renamed,
            Delta::Copied => FileChange::Copied,
            Delta::Typechange => FileChange::TypeChanged,
            _ => FileChange::Modified,
        };
        let old_path = matches!(change, FileChange::Renamed | FileChange::Copied)
            .then(|| path(delta.old_file()));
        let path = match change {
            FileChange::Deleted => path(delta.old_file()),
            _ => path(delta.new_file()),
        };
        // Binary files have no lines to count.
        let lines = match Patch::from_diff(&diff, index)? {
            Some(patch) if !patch.delta().flags().is_binary() => {
                let (_, added, removed) = patch.line_stats()?;
                Some((added, removed))
            }
            _ => None,
        };
        files.push(ChangedFile {
            path,
            old_path,
            change,
            lines,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(CommitDetails {
        commit: commit_of(&commit),
        author_email: text(author.email_bytes()),
        message: text(commit.message_bytes()).trim_end().to_owned(),
        committer: committed,
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        files,
    })
}

/// Opens the repository in the folder `repo`.
fn open(repo: &Path) -> Result<Repository, GitLogError> {
    if !repo.is_dir() {
        return Err(GitLogError::Missing(repo.to_owned()));
    }
    Repository::open(repo).map_err(|err| match err.code() {
        ErrorCode::NotFound => GitLogError::NotARepository(repo.to_owned()),
        _ => git_error(repo)(err),
    })
}

fn git_error(repo: &Path) -> impl Fn(git2::Error) -> GitLogError + '_ {
    |err| GitLogError::Git {
        path: repo.to_owned(),
        message: err.message().to_owned(),
    }
}

fn commit_of(commit: &git2::Commit) -> Commit {
    let author = commit.author();
    Commit {
        id: commit.id().to_string(),
        author: String::from_utf8_lossy(author.name_bytes()).into_owned(),
        time: time_of(&author),
        summary: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default()).into_owned(),
    }
}

/// When `signature` was made, in its time zone.
fn time_of(signature: &Signature) -> DateTime<FixedOffset> {
    let when = signature.when();
    // Broken commits with impossible times exist; show them in UTC or at
    // 1970 rather than failing.
    let offset = FixedOffset::east_opt(when.offset_minutes() * 60)
        .unwrap_or(FixedOffset::east_opt(0).expect("UTC is valid"));
    DateTime::from_timestamp(when.seconds(), 0)
        .unwrap_or_default()
        .with_timezone(&offset)
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
        let all = git_log(&dir.0, None, 0, 10).unwrap();
        assert_eq!(summaries(&all), ["Fourth", "Third", "Second", "First"]);
        assert_eq!(
            summaries(&git_log(&dir.0, None, 1, 2).unwrap()),
            ["Third", "Second"]
        );
        assert!(git_log(&dir.0, None, 4, 10).unwrap().is_empty());

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
        assert_eq!(summaries(&git_log(&dir.0, None, 0, 10).unwrap()), ["First"]);
        // A detached HEAD is read as well.
        repository.set_head_detached(first.id()).unwrap();
        assert_eq!(summaries(&git_log(&dir.0, None, 0, 10).unwrap()), ["First"]);
    }

    #[test]
    fn other_branches() {
        let dir = TempDir::new();
        let repository = repository(&dir, &["First", "Second"]);
        let head = repository.head().unwrap().peel_to_commit().unwrap();
        let first = head.parent(0).unwrap();
        repository.branch("old", &first, false).unwrap();
        repository.branch("feature/x", &head, false).unwrap();
        repository
            .reference("refs/remotes/origin/main", first.id(), false, "")
            .unwrap();
        repository
            .reference_symbolic(
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
                false,
                "",
            )
            .unwrap();
        let current = repository.head().unwrap().shorthand().unwrap().to_owned();
        let mut local = vec!["feature/x".to_owned(), "old".to_owned(), current.clone()];
        local.sort();
        let all: Vec<Branch> = (local.into_iter().map(Branch::Local))
            .chain([Branch::Remote("origin/main".to_owned())])
            .collect();
        assert_eq!(
            git_branches(&dir.0).unwrap(),
            Branches {
                all: all.clone(),
                current: Some(current),
            }
        );
        let log = |branch: Branch| {
            let commits = git_log(&dir.0, Some(&branch), 0, 10).unwrap();
            commits.into_iter().map(|c| c.summary).collect::<Vec<_>>()
        };
        assert_eq!(log(Branch::Local("old".to_owned())), ["First"]);
        assert_eq!(log(Branch::Remote("origin/main".to_owned())), ["First"]);
        assert!(git_log(&dir.0, Some(&Branch::Local("gone".to_owned())), 0, 10).is_err());

        repository.set_head_detached(first.id()).unwrap();
        assert_eq!(
            git_branches(&dir.0).unwrap(),
            Branches { all, current: None }
        );
    }

    #[test]
    fn ahead_and_behind() {
        let dir = TempDir::new();
        let repository = repository(&dir, &["First", "Second"]);
        let head = repository.head().unwrap().peel_to_commit().unwrap();
        let current = repository.head().unwrap().shorthand().unwrap().to_owned();
        assert_eq!(git_upstream(&dir.0, &current).unwrap(), None);

        // The remote has another commit after the first.
        let first = head.parent(0).unwrap();
        let author = Signature::now("Bob", "bob@example.org").unwrap();
        let theirs = repository
            .commit(
                None,
                &author,
                &author,
                "Theirs",
                &first.tree().unwrap(),
                &[&first],
            )
            .unwrap();
        repository
            .remote("origin", "https://example.org/repo.git")
            .unwrap();
        repository
            .reference("refs/remotes/origin/main", theirs, false, "")
            .unwrap();
        let mut branch = repository.find_branch(&current, BranchType::Local).unwrap();
        branch.set_upstream(Some("origin/main")).unwrap();
        assert_eq!(
            git_upstream(&dir.0, &current).unwrap(),
            Some(Upstream {
                name: "origin/main".to_owned(),
                ahead: 1,
                behind: 1,
            })
        );

        // Pruned after it was removed on the remote.
        repository
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .delete()
            .unwrap();
        assert_eq!(git_upstream(&dir.0, &current).unwrap(), None);
        assert_eq!(git_upstream(&dir.0, "gone").unwrap(), None);
    }

    #[test]
    fn uncommitted_files() {
        let dir = TempDir::new();
        let repository = Repository::init(&dir.0).unwrap();
        commit_files(
            &dir,
            &repository,
            "Start",
            &[
                ("a.txt", Some(b"a\n")),
                ("b.txt", Some(b"b\n")),
                (".gitignore", Some(b"*.log\n")),
            ],
            "Ada",
        );
        assert_eq!(git_uncommitted(&dir.0).unwrap(), Uncommitted::default());

        fs::write(dir.0.join("a.txt"), "changed\n").unwrap();
        fs::remove_file(dir.0.join("b.txt")).unwrap();
        fs::write(dir.0.join("c.txt"), "new\n").unwrap();
        fs::create_dir(dir.0.join("folder")).unwrap();
        fs::write(dir.0.join("folder/d.txt"), "new\n").unwrap();
        fs::write(dir.0.join("e.txt"), "staged\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("e.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.0.join("debug.log"), "ignored\n").unwrap();
        assert_eq!(
            git_uncommitted(&dir.0).unwrap(),
            Uncommitted { changed: 2, new: 3 }
        );
    }

    #[test]
    fn no_commits_yet() {
        let dir = TempDir::new();
        repository(&dir, &[]);
        assert!(git_log(&dir.0, None, 0, 10).unwrap().is_empty());
        let branches = git_branches(&dir.0).unwrap();
        let current = branches.current.unwrap();
        assert_eq!(branches.all, [Branch::Local(current.clone())]);
        assert_eq!(git_upstream(&dir.0, &current).unwrap(), None);
    }

    /// Commits `files` in the repository in `dir`, each written with its
    /// content or removed, by Ada or committed by `committer`.
    fn commit_files(
        dir: &TempDir,
        repository: &Repository,
        message: &str,
        files: &[(&str, Option<&[u8]>)],
        committer: &str,
    ) -> String {
        let mut index = repository.index().unwrap();
        for (path, content) in files {
            match content {
                Some(content) => {
                    fs::write(dir.0.join(path), content).unwrap();
                    index.add_path(Path::new(path)).unwrap();
                }
                None => {
                    fs::remove_file(dir.0.join(path)).unwrap();
                    index.remove_path(Path::new(path)).unwrap();
                }
            }
        }
        index.write().unwrap();
        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let time = Time::new(1_789_974_000, 120);
        let author = Signature::new("Ada", "ada@example.org", &time).unwrap();
        let committer = Signature::new(committer, "ada@example.org", &time).unwrap();
        let parent = repository
            .head()
            .ok()
            .map(|head| head.peel_to_commit().unwrap());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repository
            .commit(Some("HEAD"), &author, &committer, message, &tree, &parents)
            .unwrap()
            .to_string()
    }

    fn file(path: &str, change: FileChange, lines: Option<(usize, usize)>) -> ChangedFile {
        ChangedFile {
            path: path.to_owned(),
            old_path: None,
            change,
            lines,
        }
    }

    #[test]
    fn details_with_the_files_changed() {
        let dir = TempDir::new();
        let repository = Repository::init(&dir.0).unwrap();
        let logo: &[u8] = &[0x89, b'P', b'N', b'G', 0, 1, 2, 0, 255];
        let first = commit_files(
            &dir,
            &repository,
            "Start",
            &[
                ("a.txt", Some(b"one\ntwo\n")),
                ("old.txt", Some(b"same\ncontent\nhere\n")),
                ("logo.png", Some(logo)),
            ],
            "Ada",
        );
        let second = commit_files(
            &dir,
            &repository,
            "Change\n\nWhy it changed.\n\n",
            &[
                ("a.txt", Some(b"one\nthree\nfour\n")),
                ("b.txt", Some(b"x\n")),
                ("logo.png", None),
                ("old.txt", None),
                ("new.txt", Some(b"same\ncontent\nhere\n")),
            ],
            "Grace",
        );

        let details = git_commit(&dir.0, &first).unwrap();
        assert_eq!(details.commit.summary, "Start");
        assert_eq!(details.author_email, "ada@example.org");
        assert_eq!(details.committer, None);
        assert!(details.parents.is_empty());
        assert_eq!(
            details.files,
            [
                file("a.txt", FileChange::Added, Some((2, 0))),
                file("logo.png", FileChange::Added, None),
                file("old.txt", FileChange::Added, Some((3, 0))),
            ]
        );

        let details = git_commit(&dir.0, &second).unwrap();
        assert_eq!(details.message, "Change\n\nWhy it changed.");
        let (committer, time) = details.committer.unwrap();
        assert_eq!(committer, "Grace");
        assert_eq!(time.to_rfc3339(), "2026-09-21T09:00:00+02:00");
        assert_eq!(details.parents, [first]);
        assert_eq!(
            details.files,
            [
                file("a.txt", FileChange::Modified, Some((2, 1))),
                file("b.txt", FileChange::Added, Some((1, 0))),
                file("logo.png", FileChange::Deleted, None),
                ChangedFile {
                    old_path: Some("old.txt".to_owned()),
                    ..file("new.txt", FileChange::Renamed, Some((0, 0)))
                },
            ]
        );
    }

    #[test]
    fn missing_or_no_repository() {
        let dir = TempDir::new();
        let missing = dir.0.join("gone");
        assert_eq!(
            git_log(&missing, None, 0, 10).unwrap_err().to_string(),
            format!("the repository folder {} does not exist", missing.display())
        );
        fs::create_dir_all(&missing).unwrap();
        assert_eq!(
            git_log(&missing, None, 0, 10).unwrap_err().to_string(),
            format!("{} is not a Git repository", missing.display())
        );
    }
}
