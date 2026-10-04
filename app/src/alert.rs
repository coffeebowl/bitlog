//! Telling the user what was done or could not be done.

use adw::prelude::*;
use gettextrs::gettext;

/// Shows `heading` and `message` in a dialog over `parent`, to be closed.
pub fn show_error(parent: &impl IsA<gtk::Widget>, heading: &str, message: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("close", &gettext("_Close"));
    dialog.present(Some(parent));
}

/// The toast overlay of the window `widget` lies in, for toasts about what
/// was done.
pub fn toast_overlay(widget: &impl IsA<gtk::Widget>) -> adw::ToastOverlay {
    widget
        .ancestor(adw::ToastOverlay::static_type())
        .and_downcast()
        .expect("pages lie in the window's toast overlay")
}
