//! Commands for projects.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bitlog_core::{
    Commit, EditError, Project, ProjectCategory, ProjectSlug, ProjectStatus, Vault,
    check_repo_path, git_log, repo_name,
};
use chrono::NaiveDate;

use crate::table::{plural, table};

/// A table of all projects in their order: slug, name, category, status,
/// color and the paths of their repositories on this device.
pub fn format_projects(vault: &Vault, repos: &BTreeMap<ProjectSlug, Vec<PathBuf>>) -> String {
    let rows: Vec<[String; 6]> = vault
        .projects()
        .iter()
        .map(|project| {
            [
                project.slug.to_string(),
                project.name.clone(),
                project.category().as_str().to_owned(),
                project.status().as_str().to_owned(),
                project.color.clone(),
                repos
                    .get(&project.slug)
                    .map(|repos| {
                        let paths: Vec<String> = repos
                            .iter()
                            .map(|repo| repo.display().to_string())
                            .collect();
                        paths.join(", ")
                    })
                    .unwrap_or_default(),
            ]
        })
        .collect();
    table(&rows, &[])
}

/// The latest `limit` commits of each repository of the project `slug`,
/// below the name of the repository if there are several. A repository
/// that cannot be read, as when moved away, shows why instead.
pub fn log(vault: &Vault, slug: &ProjectSlug, limit: usize) -> Result<String> {
    if vault.project(slug).is_none() {
        return Err(EditError::UnknownProject(slug.clone()).into());
    }
    let repos = vault.repo_paths()?.remove(slug).with_context(|| {
        format!(
            "the project {slug} has no repository on this device, \
             set one with `bitlog project edit {slug} --repo FOLDER`"
        )
    })?;
    if let [repo] = repos.as_slice() {
        return Ok(format_log(&git_log(repo, None, 0, limit)?));
    }
    let mut logs = Vec::new();
    for repo in &repos {
        let log = match git_log(repo, None, 0, limit) {
            Ok(commits) => format_log(&commits),
            Err(err) => format!("error: {err}\n"),
        };
        logs.push(format!("{}\n{log}", repo_name(repo)));
    }
    Ok(logs.join("\n"))
}

/// One line per commit: short hash, time, author, tags as `git log
/// --decorate` shows them, and summary.
fn format_log(commits: &[Commit]) -> String {
    if commits.is_empty() {
        return "No commits yet.\n".to_owned();
    }
    let rows: Vec<[String; 4]> = commits
        .iter()
        .map(|commit| {
            let tags = if commit.tags.is_empty() {
                String::new()
            } else {
                let tags: Vec<String> = commit
                    .tags
                    .iter()
                    .map(|tag| format!("tag: {tag}"))
                    .collect();
                format!("({}) ", tags.join(", "))
            };
            [
                commit.short_id().to_owned(),
                commit.time.format("%Y-%m-%d %H:%M").to_string(),
                commit.author.clone(),
                format!("{tags}{}", commit.summary),
            ]
        })
        .collect();
    table(&rows, &[])
}

/// Changes to a project; `None` keeps a value as it is.
pub struct ProjectChanges {
    pub name: Option<String>,
    pub color: Option<String>,
    pub category: Option<ProjectCategory>,
    pub status: Option<ProjectStatus>,
    /// Repository folders to add.
    pub repos: Vec<PathBuf>,
    /// Repository folders to remove.
    pub no_repos: Vec<PathBuf>,
}

pub fn add_project(
    vault: &mut Vault,
    slug: ProjectSlug,
    changes: ProjectChanges,
    today: NaiveDate,
) -> Result<()> {
    let name = changes.name.clone().unwrap_or_else(|| slug.to_string());
    let repos = changed_repos(vault, &slug, &changes)?;
    let mut project = Project::new(slug, &name, today);
    apply(&mut project, changes)?;
    let slug = vault.add_project(project)?.slug.clone();
    if let Some(repos) = repos {
        vault.set_repo_paths(&slug, &repos)?;
    }
    println!("Added project {slug}");
    Ok(())
}

pub fn edit_project(vault: &mut Vault, slug: &ProjectSlug, changes: ProjectChanges) -> Result<()> {
    let repos = changed_repos(vault, slug, &changes)?;
    vault.update_project(slug, |project| apply(project, changes))?;
    if let Some(repos) = repos {
        vault.set_repo_paths(slug, &repos)?;
    }
    Ok(())
}

pub fn rename_project(vault: &mut Vault, slug: &ProjectSlug, new_slug: &ProjectSlug) -> Result<()> {
    let (days, notes) = vault.rename_project(slug, new_slug)?;
    println!(
        "Renamed project {slug} to {new_slug}, changed {} and {}",
        plural(days, "day"),
        plural(notes, "note")
    );
    Ok(())
}

/// The repositories of the project `slug` with the changes of `changes`, if
/// any, checked before anything is saved, so that an invalid path changes
/// nothing. Removing a repository the project lacks is fine.
fn changed_repos(
    vault: &Vault,
    slug: &ProjectSlug,
    changes: &ProjectChanges,
) -> Result<Option<Vec<PathBuf>>> {
    if changes.repos.is_empty() && changes.no_repos.is_empty() {
        return Ok(None);
    }
    let mut repos = vault.repo_paths()?.remove(slug).unwrap_or_default();
    for repo in &changes.repos {
        check_repo_path(repo)?;
        if !repos.contains(repo) {
            repos.push(repo.clone());
        }
    }
    repos.retain(|repo| !changes.no_repos.contains(repo));
    Ok(Some(repos))
}

fn apply(project: &mut Project, changes: ProjectChanges) -> Result<(), EditError> {
    if let Some(name) = changes.name {
        project.name = name;
    }
    if let Some(color) = changes.color {
        // `#` starts a comment in the shell, so it may be left out.
        let color = if color.starts_with('#') {
            color
        } else {
            format!("#{color}")
        };
        project.set_color(&color)?;
    }
    if let Some(category) = changes.category {
        project.set_category(category);
    }
    if let Some(status) = changes.status {
        project.set_status(status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn log_lines_with_tags() {
        let commit = |id: &str, author: &str, summary: &str, tags: &[&str]| Commit {
            id: id.repeat(40),
            author: author.to_owned(),
            time: "2026-09-21T09:00:00+02:00".parse().unwrap(),
            summary: summary.to_owned(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        };
        assert_eq!(
            format_log(&[
                commit("a", "Ada", "Release", &["26.09", "v1.0"]),
                commit("b", "Grace", "Start", &[]),
            ]),
            "aaaaaaa  2026-09-21 09:00  Ada    (tag: 26.09, tag: v1.0) Release\n\
             bbbbbbb  2026-09-21 09:00  Grace  Start\n"
        );
    }

    #[test]
    fn sample_projects() {
        let vault =
            Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
                .unwrap();
        assert_eq!(
            format_projects(
                &vault,
                &BTreeMap::from([(
                    "infra".parse().unwrap(),
                    vec!["/code/infra".into(), "/code/proto".into()]
                )])
            ),
            "webshop   Webshop         work   active  #3584e4\n\
             infra     Infrastructure  work   active  #2ec27e  /code/infra, /code/proto\n\
             meetings  Meetings        work   active  #f6d32d\n\
             filler    Filler          duty   active  #c061cb\n\
             pause     Break           break  active  #9a9996\n"
        );
    }
}
