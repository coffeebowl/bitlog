//! The actions on a note, `note.*`, that the notes page and the project page
//! share: opening it, following its links, renaming and deleting it, and
//! opening its file elsewhere.

use std::future::Future;
use std::rc::Rc;

use adw::prelude::*;
use bitlog_core::{NotePath, Vault};
use gtk::{gio, glib};

use crate::launch;
use crate::note_dialogs;
use crate::note_view::NoteView;
use crate::search_index::SearchIndex;
use crate::widgets::param;

/// A page that lists notes and shows one in its `NoteView`. It has the
/// signal `days-changed`, for links in day files changed by renaming.
pub trait NoteHost: IsA<gtk::Widget> {
    fn vault(&self) -> Rc<Vault>;
    fn note_view(&self) -> NoteView;
    /// Where the links to a note lie, before it is renamed.
    fn index(&self) -> SearchIndex;
    fn open_note(&self, note: &NotePath);
    /// Goes back from the note shown, which is gone.
    fn close_note(&self);
    /// Lists the notes again, as one was added, renamed or deleted.
    fn refresh_notes(&self);
}

/// Adds the actions on notes to `host`.
pub fn add<T: NoteHost>(host: &T) {
    let group = gio::SimpleActionGroup::new();
    add_action(&group, host, "open", |host, note| async move {
        host.open_note(&note);
    });
    add_action(&group, host, "follow", |host, note| async move {
        let vault = host.vault();
        if let Some(note) = note_dialogs::follow_link(&host, &vault, &note).await {
            // It may be new.
            host.refresh_notes();
            host.open_note(&note);
        }
    });
    add_action(&group, host, "rename", |host, note| async move {
        rename(&host, note).await;
    });
    add_action(&group, host, "delete", |host, note| async move {
        if note_dialogs::delete_note(&host, &host.vault(), &note).await {
            if host.note_view().note() == Some(note) {
                host.close_note();
            }
            host.refresh_notes();
        }
    });
    // With what is being typed saved, for the other app to see.
    add_action(&group, host, "open-file", |host, note| async move {
        host.note_view().save_now();
        launch::open_file(&host, &host.vault().note_path(&note)).await;
    });
    add_action(&group, host, "show-file", |host, note| async move {
        host.note_view().save_now();
        launch::show_in_folder(&host, &host.vault().note_path(&note)).await;
    });
    host.insert_action_group("note", Some(&group));
}

/// Adds the action `name`, which runs `run` with the note it is called with.
fn add_action<T: NoteHost, F: Future<Output = ()> + 'static>(
    group: &gio::SimpleActionGroup,
    host: &T,
    name: &str,
    run: impl Fn(T, NotePath) -> F + 'static,
) {
    let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
    let host = host.downgrade();
    action.connect_activate(move |_, note| {
        if let Some(host) = host.upgrade() {
            glib::spawn_future_local(run(host, param(note, "notes")));
        }
    });
    group.add_action(&action);
}

/// Asks for a new name of `note` and renames it, showing it under that name
/// if it is shown.
async fn rename<T: NoteHost>(host: &T, note: NotePath) {
    let view = host.note_view();
    // Renaming may change the links in the note shown, and the index has to
    // find what was just typed.
    view.save_now();
    let renamed = note_dialogs::rename_note(host, &host.vault(), &host.index(), &note).await;
    let Some((renamed, updated_links)) = renamed else {
        return;
    };
    if updated_links {
        host.emit_by_name::<()>("days-changed", &[]);
    }
    host.refresh_notes();
    if view.note() == Some(note) {
        view.forget();
        host.open_note(&renamed);
    } else {
        view.reload();
    }
}
