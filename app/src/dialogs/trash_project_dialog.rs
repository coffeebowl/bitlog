//! Asking before a project goes to the trash, and where its blocks go.

use adw::prelude::*;
use bitlog_core::{Project, ProjectSlug, Vault};
use gettextrs::{gettext, ngettext};

use crate::format::plural;

/// Asks whether to move `project` of `vault`, which has `blocks` blocks, to
/// the trash, and which project its blocks move to. Returns `None` if the
/// user cancels, else the project for the blocks, if it has any.
pub async fn ask_trash_project(
    parent: &impl IsA<gtk::Widget>,
    vault: &Vault,
    project: &Project,
    blocks: usize,
) -> Option<Option<ProjectSlug>> {
    let mut body = gettext("“{name}” will be moved to the trash with its notes, files and tasks.")
        .replace("{name}", &project.name);
    let others: Vec<&Project> = vault
        .projects()
        .iter()
        .filter(|other| other.slug != project.slug)
        .collect();
    let choice = (blocks > 0).then(|| {
        body.push_str("\n\n");
        body.push_str(
            &ngettext(
                "Its {count} block will move to the project chosen below. If it doesn’t belong there, move it by hand first; the project page lists it.",
                "Its {count} blocks will move to the project chosen below. If they don’t all belong there, move them by hand first; the project page lists them.",
                plural(blocks),
            )
            .replace("{count}", &blocks.to_string()),
        );
        let names: Vec<&str> = others.iter().map(|other| other.name.as_str()).collect();
        adw::ComboRow::builder()
            .title(gettext("Move Blocks To"))
            .model(&gtk::StringList::new(&names))
            .build()
    });
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Move Project to Trash?"))
        .body(body)
        .close_response("cancel")
        .default_response("cancel")
        .build();
    if let Some(row) = &choice {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        list.append(row);
        dialog.set_extra_child(Some(&list));
    }
    dialog.add_responses(&[
        ("cancel", &gettext("_Cancel")),
        ("trash", &gettext("Move to _Trash")),
    ]);
    dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
    // Blocks need another project to move to.
    dialog.set_response_enabled("trash", blocks == 0 || !others.is_empty());
    if dialog.choose_future(Some(parent)).await != "trash" {
        return None;
    }
    Some(choice.and_then(|row| {
        let index = usize::try_from(row.selected()).ok()?;
        others.get(index).map(|other| other.slug.clone())
    }))
}
