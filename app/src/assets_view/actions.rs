//! What the assets view does with files and folders.

use std::fs;
use std::path::PathBuf;

use adw::prelude::*;
use bitlog_core::{AssetPath, Vault};
use gettextrs::gettext;
use gtk::gio;

use super::AssetsView;
use crate::alert::show_error;
use crate::dialogs::note_dialogs::ask_name;
use crate::launch;
use crate::trash::trash;

impl AssetsView {
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

    /// Copies `files`, which may be folders, into the folder shown in the
    /// background, then shows them. Files that cannot be added are named in
    /// one message.
    pub(super) async fn add_assets(&self, files: &[gio::File]) {
        let Some(slug) = self.slug() else {
            return;
        };
        let folder = self.folder();
        // A copy for the thread copying.
        let vault = Vault::clone(&self.vault());
        let paths: Vec<Option<PathBuf>> = files.iter().map(|file| file.path()).collect();
        let errors = gio::spawn_blocking(move || {
            paths
                .iter()
                .filter_map(|path| match path {
                    Some(path) => vault
                        .add_asset(&slug, folder.as_ref(), path)
                        .err()
                        .map(|err| err.to_string()),
                    None => Some(not_local()),
                })
                .collect::<Vec<_>>()
        })
        .await
        .expect("copying files does not panic");
        self.reload();
        if !errors.is_empty() {
            show_error(self, &gettext("Cannot Add Files"), &errors.join("\n"));
        }
    }

    pub(super) async fn new_folder(&self) {
        let Some(slug) = self.slug() else {
            return;
        };
        let folder = self.folder();
        let (valid_slug, valid_folder) = (slug.clone(), folder.clone());
        let Some(name) = ask_name(
            self,
            &gettext("New Folder"),
            &gettext("_Create"),
            "",
            move |name| AssetPath::in_folder(&valid_slug, valid_folder.as_ref(), name).is_ok(),
        )
        .await
        else {
            return;
        };
        if let Err(err) = self
            .vault()
            .create_asset_folder(&slug, folder.as_ref(), &name)
        {
            show_error(self, &gettext("Cannot Create Folder"), &err.to_string());
        }
        self.reload();
    }

    /// Opens the folder shown in the file manager, creating the assets
    /// folder of the project first, so that files can be put there.
    pub(super) async fn open_folder(&self) {
        let Some(slug) = self.slug() else {
            return;
        };
        let folder = self
            .vault()
            .asset_folder_path(&slug, self.folder().as_ref());
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
        let is_folder = self.vault().asset_path(&asset).is_dir();
        let valid = asset.clone();
        let Some(name) = ask_name(
            self,
            &if is_folder {
                gettext("Rename Folder")
            } else {
                gettext("Rename File")
            },
            &gettext("_Rename"),
            asset.name(),
            move |name| valid.with_name(name).is_ok(),
        )
        .await
        else {
            return;
        };
        if let Err(err) = self.vault().rename_asset(&asset, &name) {
            let heading = if is_folder {
                gettext("Cannot Rename Folder")
            } else {
                gettext("Cannot Rename File")
            };
            show_error(self, &heading, &err.to_string());
        }
        self.reload();
    }

    pub(super) async fn trash_asset(&self, asset: AssetPath) {
        let path = self.vault().asset_path(&asset);
        let trashed = if path.is_dir() {
            gettext("Folder moved to trash")
        } else {
            gettext("File moved to trash")
        };
        trash(self, &path, &trashed).await;
        self.reload();
    }
}

/// The message for a file chosen or dropped that has no local path.
fn not_local() -> String {
    gettext("The file is not on a local file system.")
}
