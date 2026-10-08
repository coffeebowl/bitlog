//! Opening files of the vault in other apps.

use std::path::Path;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::alert::show_error;

/// Opens `path` in the app the system chooses for it.
pub async fn open_file(parent: &impl IsA<gtk::Widget>, path: &Path) {
    let window = parent.root().and_downcast::<gtk::Window>();
    let launched = gtk::FileLauncher::new(Some(&gio::File::for_path(path)))
        .launch_future(window.as_ref())
        .await;
    show_launch_error(parent, &gettext("Cannot Open File"), launched);
}

/// Shows the folder of `path` in the file manager, with the file selected.
pub async fn show_in_folder(parent: &impl IsA<gtk::Widget>, path: &Path) {
    let window = parent.root().and_downcast::<gtk::Window>();
    let launched = gtk::FileLauncher::new(Some(&gio::File::for_path(path)))
        .open_containing_folder_future(window.as_ref())
        .await;
    show_launch_error(parent, &gettext("Cannot Show File"), launched);
}

/// Shows the error of launching an app, unless the user dismissed it.
pub fn show_launch_error(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    launched: Result<(), glib::Error>,
) {
    if let Err(err) = launched
        && !err.matches(gtk::DialogError::Dismissed)
        && !err.matches(gtk::DialogError::Cancelled)
    {
        show_error(parent, heading, err.message());
    }
}
