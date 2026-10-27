//! Choosing a project in a popover.

use adw::prelude::*;
use bitlog_core::{Project, ProjectSlug, ProjectStatus, Vault};
use gtk::glib;

use crate::colors::dot_markup;

/// A popover with the projects of `vault` that are not archived, in their
/// order. Choosing one closes it and calls `on_chosen`.
pub fn project_popover(vault: &Vault, on_chosen: impl Fn(ProjectSlug) + 'static) -> gtk::Popover {
    popover(vault, None, move |slug| {
        on_chosen(slug.expect("only projects are offered"));
    })
}

/// Like `project_popover`, with a first row named `none` for no project,
/// passed as `None`.
pub fn optional_project_popover(
    vault: &Vault,
    none: &str,
    on_chosen: impl Fn(Option<ProjectSlug>) + 'static,
) -> gtk::Popover {
    popover(vault, Some(none), on_chosen)
}

fn popover(
    vault: &Vault,
    none: Option<&str>,
    on_chosen: impl Fn(Option<ProjectSlug>) + 'static,
) -> gtk::Popover {
    let projects = vault
        .projects()
        .iter()
        .filter(|project| project.status() != ProjectStatus::Archived);
    let mut choices: Vec<(String, Option<ProjectSlug>)> = none
        .map(|none| (none_markup(none), None))
        .into_iter()
        .collect();
    choices.extend(projects.map(|project| (project_markup(project), Some(project.slug.clone()))));

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    for (markup, _) in &choices {
        list.append(&choice_row(markup));
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(360)
        .child(&list)
        .build();
    let popover = gtk::Popover::builder().child(&scrolled).build();

    let slugs: Vec<Option<ProjectSlug>> = choices.into_iter().map(|(_, slug)| slug).collect();
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

/// `none`, the choice of no project, behind a dot in the colour of the
/// text, as Pango markup. So it lines up with the projects and still differs
/// from gray ones.
pub fn none_markup(none: &str) -> String {
    format!("● {}", glib::markup_escape_text(none))
}

/// The name of `project` behind a dot in its colour, as Pango markup.
pub fn project_markup(project: &Project) -> String {
    dot_markup(&project.color, &project.name)
}

fn choice_row(markup: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(markup)
        .use_markup(true)
        .xalign(0.0)
        .build()
}
