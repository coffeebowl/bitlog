//! Telling the user that something could not be done.

use adw::prelude::*;
use gettextrs::gettext;

/// Shows `heading` and `message` in a dialog over `parent`, to be closed.
pub fn show_error(parent: &impl IsA<gtk::Widget>, heading: &str, message: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("close", &gettext("_Close"));
    dialog.present(Some(parent));
}
