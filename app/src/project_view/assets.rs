//! What the project view does with the assets of the project.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{AssetPath, Vault};
use gettextrs::gettext;
use gtk::{gio, glib};

use super::ProjectView;
use crate::alert::show_error;
use crate::launch;
use crate::note_dialogs::ask_name;

impl ProjectView {
    /// The vault shown, which the actions on assets need.
    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("assets are only shown with a vault")
    }

    pub(super) async fn choose_assets(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Add Files"))
            .modal(true)
            .build();
        let window = self.root().and_downcast::<gtk::Window>();
        // Dismissing the dialog is reported as an error, too.
        let Ok(files) = dialog.open_multiple_future(window.as_ref()).await else {
            return;
        };
        let files: Vec<gio::File> = files.iter().filter_map(Result::ok).collect();
        self.add_assets(&files).await;
    }

    /// Copies `files` into the assets of the project in the background, then
    /// shows them. Files that cannot be added are named in one message.
    pub(super) async fn add_assets(&self, files: &[gio::File]) {
        let Some(slug) = self.slug() else {
            return;
        };
        // A copy for the thread copying.
        let vault = Vault::clone(&self.vault());
        let paths: Vec<Option<PathBuf>> = files.iter().map(|file| file.path()).collect();
        let errors = gio::spawn_blocking(move || {
            paths
                .iter()
                .filter_map(|path| match path {
                    Some(path) => vault
                        .add_asset(&slug, path)
                        .err()
                        .map(|err| err.to_string()),
                    None => Some(not_local()),
                })
                .collect::<Vec<_>>()
        })
        .await
        .expect("copying files does not panic");
        self.show_assets();
        if !errors.is_empty() {
            show_error(self, &gettext("Cannot Add Files"), &errors.join("\n"));
        }
    }

    /// Opens the assets folder of the project in the file manager, creating
    /// it first, so that files can be put there.
    pub(super) async fn open_assets_folder(&self) {
        let Some(slug) = self.slug() else {
            return;
        };
        let folder = self.vault().assets_folder(&slug);
        if let Err(err) = fs::create_dir_all(&folder) {
            let message = format!("{}: {err}", folder.display());
            show_error(self, &gettext("Cannot Open Folder"), &message);
            return;
        }
        launch::open_file(self, &folder).await;
    }

    /// Opens `asset` with the app the system chooses for it.
    pub(super) async fn open_asset(&self, asset: AssetPath) {
        launch::open_file(self, &self.vault().asset_path(&asset)).await;
    }

    pub(super) async fn show_asset_in_folder(&self, asset: AssetPath) {
        launch::show_in_folder(self, &self.vault().asset_path(&asset)).await;
    }

    pub(super) async fn rename_asset(&self, asset: AssetPath) {
        let valid = asset.clone();
        let Some(name) = ask_name(
            self,
            &gettext("Rename File"),
            &gettext("_Rename"),
            asset.name(),
            move |name| valid.with_name(name).is_ok(),
        )
        .await
        else {
            return;
        };
        if let Err(err) = self.vault().rename_asset(&asset, &name) {
            show_error(self, &gettext("Cannot Rename File"), &err.to_string());
        }
        self.show_assets();
    }

    pub(super) async fn trash_asset(&self, asset: AssetPath) {
        let file = gio::File::for_path(self.vault().asset_path(&asset));
        if let Err(err) = file.trash_future(glib::Priority::DEFAULT).await {
            show_error(self, &gettext("Cannot Move File to Trash"), err.message());
        }
        self.show_assets();
    }
}

/// The message for a file chosen or dropped that has no local path.
fn not_local() -> String {
    gettext("The file is not on a local file system.")
}
