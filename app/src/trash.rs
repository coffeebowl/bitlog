//! Moving files of the vault to the trash.

use std::path::Path;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::alert::{show_error, toast_overlay};

/// Moves the file at `path` to the trash and says so with the toast
/// `trashed`. Where there is no trash, as on some drives, it asks whether to
/// delete the file for good instead. Returns whether the file is gone;
/// failing to delete it is shown.
pub async fn trash(parent: &impl IsA<gtk::Widget>, path: &Path, trashed: &str) -> bool {
    let file = gio::File::for_path(path);
    if file.trash_future(glib::Priority::DEFAULT).await.is_ok() {
        toast_overlay(parent).add_toast(adw::Toast::new(trashed));
        return true;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let body = gettext("“{name}” cannot be moved to the trash.").replace("{name}", &name);
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Delete Permanently?"))
        .body(body)
        .close_response("cancel")
        .default_response("cancel")
        .build();
    dialog.add_responses(&[
        ("cancel", &gettext("_Cancel")),
        ("delete", &gettext("_Delete")),
    ]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    if dialog.choose_future(Some(parent)).await != "delete" {
        return false;
    }
    file.delete_future(glib::Priority::DEFAULT)
        .await
        .inspect_err(|err| show_error(parent, &gettext("Cannot Delete File"), err.message()))
        .is_ok()
}
