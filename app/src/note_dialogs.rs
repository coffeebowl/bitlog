//! The questions around notes that the project and note pages share, and
//! asking for a name, which assets use as well.

use adw::prelude::*;
use bitlog_core::{NotePath, ProjectSlug, Vault};
use bitlog_index::Found;
use gettextrs::{gettext, ngettext};
use gtk::glib;

use crate::alert::show_error;
use crate::search_index::SearchIndex;

/// Asks for the name of a note in `project`, starting with `name`.
/// Returns `None` if the user cancels.
pub async fn ask_note_name(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    accept: &str,
    project: &ProjectSlug,
    name: &str,
) -> Option<String> {
    let project = project.clone();
    ask_name(parent, heading, accept, name, move |text| {
        NotePath::new(project.clone(), text).is_ok()
    })
    .await
}

/// Asks for a new name instead of `name` that `is_valid` accepts, as for
/// a note or an asset. Returns `None` if the user cancels.
pub async fn ask_name(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    accept: &str,
    name: &str,
    is_valid: impl Fn(&str) -> bool + 'static,
) -> Option<String> {
    let entry = gtk::Entry::builder()
        .text(name)
        .activates_default(true)
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&entry)
        .close_response("cancel")
        .default_response("accept")
        .focus_widget(&entry)
        .build();
    dialog.add_responses(&[("cancel", &gettext("_Cancel")), ("accept", accept)]);
    dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
    let is_new_name = {
        let name = name.to_owned();
        move |text: &str| {
            let text = text.trim();
            text != name && is_valid(text)
        }
    };
    dialog.set_response_enabled("accept", false);
    entry.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |entry| dialog.set_response_enabled("accept", is_new_name(&entry.text()))
    ));
    let response = dialog.choose_future(Some(parent)).await;
    (response == "accept").then(|| entry.text().trim().to_owned())
}

/// Asks whether the note `note`, which a link points to, should be created.
pub async fn confirm_create(
    parent: &impl IsA<gtk::Widget>,
    vault: &Vault,
    note: &NotePath,
) -> bool {
    let project = vault.project_name(note.project());
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Create Note?"))
        .body(
            gettext("There is no note “{name}” in {project} yet.")
                .replace("{name}", note.name())
                .replace("{project}", project),
        )
        .close_response("cancel")
        .default_response("create")
        .build();
    dialog.add_responses(&[
        ("cancel", &gettext("_Cancel")),
        ("create", &gettext("_Create")),
    ]);
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.choose_future(Some(parent)).await == "create"
}

/// Asks whether the note `note` should be permanently deleted.
pub async fn confirm_delete(parent: &impl IsA<gtk::Widget>, note: &NotePath) -> bool {
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Delete Note?"))
        .body(gettext("“{name}” will be permanently deleted.").replace("{name}", note.name()))
        .close_response("cancel")
        .default_response("cancel")
        .build();
    dialog.add_responses(&[
        ("cancel", &gettext("_Cancel")),
        ("delete", &gettext("_Delete")),
    ]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.choose_future(Some(parent)).await == "delete"
}

/// Asks for a new name of `note` and, if other places link to it, whether
/// the links should follow, then renames it. Returns the renamed note and
/// whether links were updated, or `None` if the user cancels or renaming
/// fails, which is shown. What is being typed has to be saved before, so
/// that the index finds it.
pub async fn rename_note(
    parent: &impl IsA<gtk::Widget>,
    vault: &Vault,
    index: &SearchIndex,
    note: &NotePath,
) -> Option<(NotePath, bool)> {
    let name = ask_note_name(
        parent,
        &gettext("Rename Note"),
        &gettext("_Rename"),
        note.project(),
        note.name(),
    )
    .await?;
    let linking = match index.backlinks(vault, note.clone()).await {
        Ok(linking) => linking,
        Err(err) => {
            show_error(parent, &gettext("Cannot Rename Note"), &err.to_string());
            return None;
        }
    };
    let itself = Found::Note(note.clone());
    let others = linking.iter().filter(|link| link.found != itself).count();
    // Only the note itself links to it, if at all.
    let update_links = others == 0 || ask_update_links(parent, note, others).await?;
    match vault.rename_note(note, &name, update_links) {
        Ok(renamed) => Some((renamed, update_links)),
        Err(err) => {
            show_error(parent, &gettext("Cannot Rename Note"), &err.to_string());
            None
        }
    }
}

/// Asks whether the links in `others` other notes, day notes and blocks
/// to `note` should point to its new name. Returns `None` if the user
/// cancels.
async fn ask_update_links(
    parent: &impl IsA<gtk::Widget>,
    note: &NotePath,
    others: usize,
) -> Option<bool> {
    let count = u32::try_from(others).unwrap_or(u32::MAX);
    let body = ngettext(
        "{count} other place links to “{name}”. Should the link point to the new name?",
        "{count} other places link to “{name}”. Should the links point to the new name?",
        count,
    )
    .replace("{count}", &others.to_string())
    .replace("{name}", note.name());
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Update Links?"))
        .body(body)
        .close_response("cancel")
        .default_response("update")
        .build();
    dialog.add_responses(&[
        ("cancel", &gettext("_Cancel")),
        ("keep", &gettext("_Keep Links")),
        ("update", &gettext("_Update Links")),
    ]);
    dialog.set_response_appearance("update", adw::ResponseAppearance::Suggested);
    match dialog.choose_future(Some(parent)).await.as_str() {
        "update" => Some(true),
        "keep" => Some(false),
        _ => None,
    }
}
