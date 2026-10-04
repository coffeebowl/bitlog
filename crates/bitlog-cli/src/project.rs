//! Commands for projects.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bitlog_core::{
    Commit, EditError, Project, ProjectSlug, ProjectStatus, Vault, check_repo_path, git_log,
};
use chrono::NaiveDate;

use crate::table::table;

/// A table of all projects in their order: slug, name, category, status,
/// color and the path of their repository on this device.
pub fn format_projects(vault: &Vault, repos: &BTreeMap<ProjectSlug, PathBuf>) -> String {
    let rows: Vec<[String; 6]> = vault
        .projects()
        .iter()
        .map(|project| {
            [
                project.slug.to_string(),
                project.name.clone(),
                project.category.clone(),
                project.status.as_str().to_owned(),
                project.color.clone(),
                repos
                    .get(&project.slug)
                    .map(|repo| repo.display().to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    table(&rows, &[])
}

/// The latest `limit` commits of the repository of the project `slug`.
pub fn log(vault: &Vault, slug: &ProjectSlug, limit: usize) -> Result<String> {
    if vault.project(slug).is_none() {
        return Err(EditError::UnknownProject(slug.clone()).into());
    }
    let repo = vault.repo_paths()?.remove(slug).with_context(|| {
        format!(
            "the project {slug} has no repository on this device, \
             set one with `bitlog project edit {slug} --repo FOLDER`"
        )
    })?;
    Ok(format_log(&git_log(&repo, None, 0, limit)?))
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
    pub category: Option<String>,
    pub status: Option<ProjectStatus>,
    /// `Some(None)` removes the repository.
    pub repo: Option<Option<PathBuf>>,
}

pub fn add_project(
    vault: &mut Vault,
    slug: ProjectSlug,
    changes: ProjectChanges,
    today: NaiveDate,
) -> Result<()> {
    let name = changes.name.clone().unwrap_or_else(|| slug.to_string());
    let repo = checked_repo(&changes)?;
    let mut project = Project::new(slug, &name, today);
    apply(&mut project, changes)?;
    let slug = vault.add_project(project)?.slug.clone();
    if let Some(repo) = repo {
        vault.set_repo_path(&slug, repo.as_deref())?;
    }
    println!("Added project {slug}");
    Ok(())
}

pub fn edit_project(vault: &mut Vault, slug: &ProjectSlug, changes: ProjectChanges) -> Result<()> {
    let repo = checked_repo(&changes)?;
    vault.update_project(slug, |project| apply(project, changes))?;
    if let Some(repo) = repo {
        vault.set_repo_path(slug, repo.as_deref())?;
    }
    Ok(())
}

pub fn rename_project(vault: &mut Vault, slug: &ProjectSlug, new_slug: &ProjectSlug) -> Result<()> {
    let (days, notes) = vault.rename_project(slug, new_slug)?;
    let plural = |count: usize, noun: &str| match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    };
    println!(
        "Renamed project {slug} to {new_slug}, changed {} and {}",
        plural(days, "day"),
        plural(notes, "note")
    );
    Ok(())
}

/// The repository change of `changes`, checked before anything is saved,
/// so that an invalid path changes nothing.
fn checked_repo(changes: &ProjectChanges) -> Result<Option<Option<PathBuf>>, EditError> {
    if let Some(Some(repo)) = &changes.repo {
        check_repo_path(repo)?;
    }
    Ok(changes.repo.clone())
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
        project.category = category;
    }
    if let Some(status) = changes.status {
        project.status = status;
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
                &BTreeMap::from([("infra".parse().unwrap(), "/code/infra".into())])
            ),
            "webshop   Webshop         work      active  #3584e4\n\
             infra     Infrastructure  work      active  #2ec27e  /code/infra\n\
             meetings  Meetings        overhead  active  #f6d32d\n\
             filler    Filler          overhead  active  #c061cb\n\
             pause     Break           break     active  #9a9996\n"
        );
    }
}
