//! Commands for projects.

use anyhow::Result;
use chrono::NaiveDate;
use knotbook_core::{EditError, Project, ProjectSlug, ProjectStatus, Vault};

/// A table of all projects: slug, name, category, status, color and whether
/// they are pinned.
pub fn format_projects(vault: &Vault) -> String {
    let rows: Vec<[&str; 6]> = vault
        .projects()
        .iter()
        .map(|project| {
            [
                project.slug.as_str(),
                &project.name,
                &project.category,
                project.status.as_str(),
                &project.color,
                if project.pinned { "pinned" } else { "" },
            ]
        })
        .collect();
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2), width(3)];
    rows.iter()
        .map(|[slug, name, category, status, color, pinned]| {
            let line = format!(
                "{slug:<0$}  {name:<1$}  {category:<2$}  {status:<3$}  {color}  {pinned}",
                widths[0], widths[1], widths[2], widths[3]
            );
            format!("{}\n", line.trim_end())
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
}

pub fn add_project(
    vault: &mut Vault,
    slug: ProjectSlug,
    changes: ProjectChanges,
    today: NaiveDate,
) -> Result<()> {
    let name = changes.name.clone().unwrap_or_else(|| slug.to_string());
    let mut project = Project::new(slug, &name, today);
    apply(&mut project, changes)?;
    let project = vault.add_project(project)?;
    println!("Added project {}", project.slug);
    Ok(())
}

pub fn edit_project(vault: &mut Vault, slug: &ProjectSlug, changes: ProjectChanges) -> Result<()> {
    vault.update_project(slug, |project| apply(project, changes))?;
    Ok(())
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
            format_projects(&vault),
            "filler    Filler          overhead  active  #c061cb  pinned\n\
             infra     Infrastructure  work      active  #2ec27e\n\
             meetings  Meetings        overhead  active  #f6d32d  pinned\n\
             pause     Break           break     active  #9a9996  pinned\n\
             webshop   Webshop         work      active  #3584e4  pinned\n"
        );
    }
}
