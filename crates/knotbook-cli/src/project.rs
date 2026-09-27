//! Commands for projects.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use knotbook_core::{
    Commit, EditError, Project, ProjectSlug, ProjectStatus, Vault, check_repo_path, git_log,
};

/// A table of all projects: slug, name, category, status, color, whether
/// they are pinned and the path of their repository on this device.
pub fn format_projects(vault: &Vault, repos: &BTreeMap<ProjectSlug, PathBuf>) -> String {
    let rows: Vec<[String; 7]> = vault
        .projects()
        .iter()
        .map(|project| {
            [
                project.slug.to_string(),
                project.name.clone(),
                project.category.clone(),
                project.status.as_str().to_owned(),
                project.color.clone(),
                if project.pinned { "pinned" } else { "" }.to_owned(),
                repos
                    .get(&project.slug)
                    .map(|repo| repo.display().to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..6)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let mut line: String = row
                .iter()
                .zip(&widths)
                .map(|(value, width)| format!("{value:<width$}  "))
                .collect();
            line.push_str(&row[6]);
            format!("{}\n", line.trim_end())
        })
        .collect()
}

/// The latest `limit` commits of the repository of the project `slug`.
pub fn log(vault: &Vault, slug: &ProjectSlug, limit: usize) -> Result<String> {
    if vault.project(slug).is_none() {
        return Err(EditError::UnknownProject(slug.clone()).into());
    }
    let repo = vault.repo_paths()?.remove(slug).with_context(|| {
        format!(
            "the project {slug} has no repository on this device, \
             set one with `knotbook project edit {slug} --repo FOLDER`"
        )
    })?;
    Ok(format_log(&git_log(&repo, 0, limit)?))
}

/// One line per commit: short hash, time, author and summary.
fn format_log(commits: &[Commit]) -> String {
    if commits.is_empty() {
        return "No commits yet.\n".to_owned();
    }
    let width = commits
        .iter()
        .map(|commit| commit.author.chars().count())
        .max()
        .unwrap_or(0);
    commits
        .iter()
        .map(|commit| {
            format!(
                "{}  {}  {:<width$}  {}\n",
                commit.short_id(),
                commit.time.format("%Y-%m-%d %H:%M"),
                commit.author,
                commit.summary
            )
        })
        .collect()
}

/// Changes to a project; `None` keeps a value as it is.
pub struct ProjectChanges {
    pub name: Option<String>,
    pub color: Option<String>,
    pub category: Option<String>,
    pub status: Option<ProjectStatus>,
    pub pinned: Option<bool>,
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
    if let Some(pinned) = changes.pinned {
        project.pinned = pinned;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

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
            "filler    Filler          overhead  active  #c061cb  pinned\n\
             infra     Infrastructure  work      active  #2ec27e          /code/infra\n\
             meetings  Meetings        overhead  active  #f6d32d  pinned\n\
             pause     Break           break     active  #9a9996  pinned\n\
             webshop   Webshop         work      active  #3584e4  pinned\n"
        );
    }
}
