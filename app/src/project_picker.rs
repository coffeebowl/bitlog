//! Choosing a project in a popover.

use adw::prelude::*;
use gtk::glib;
use knotbook_core::{Project, ProjectSlug, ProjectStatus, Vault};

use crate::colors::color_dot;

/// A popover with the projects of `vault` that are not archived, pinned ones
/// first. Choosing one closes it and calls `on_chosen`.
pub fn project_popover(vault: &Vault, on_chosen: impl Fn(ProjectSlug) + 'static) -> gtk::Popover {
    let mut projects: Vec<&Project> = vault
        .projects()
        .iter()
        .filter(|project| project.status != ProjectStatus::Archived)
        .collect();
    projects.sort_by_key(|project| !project.pinned);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    for project in &projects {
        list.append(&project_row(project));
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(360)
        .child(&list)
        .build();
    let popover = gtk::Popover::builder().child(&scrolled).build();

    let slugs: Vec<ProjectSlug> = projects
        .iter()
        .map(|project| project.slug.clone())
        .collect();
    list.connect_row_activated(glib::clone!(
        #[weak]
        popover,
        move |_, row| {
            let index = usize::try_from(row.index()).expect("an activated row is in the list");
            popover.popdown();
            on_chosen(slugs[index].clone());
        }
    ));
    popover
}

/// The name of `project` behind a dot in its colour, as Pango markup.
pub fn project_markup(project: &Project) -> String {
    format!(
        "{} {}",
        color_dot(&project.color),
        glib::markup_escape_text(&project.name),
    )
}

fn project_row(project: &Project) -> gtk::Label {
    gtk::Label::builder()
        .label(project_markup(project))
        .use_markup(true)
        .xalign(0.0)
        .build()
}
