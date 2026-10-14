//! What the contacts page does with the picture of the contact shown.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, Vault};
use gettextrs::gettext;
use gtk::gio;

use super::ContactsPage;
use crate::alert::show_error;
use crate::contact_image::{Source, jpeg};
use crate::trash::trash;

impl ContactsPage {
    pub(super) async fn choose_picture(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Pictures")));
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose Picture"))
            .filters(&filters)
            .modal(true)
            .build();
        let window = self.root().and_downcast::<gtk::Window>();
        // Dismissing the dialog is reported as an error, too.
        let Ok(file) = dialog.open_future(window.as_ref()).await else {
            return;
        };
        match file.path() {
            Some(path) => self.set_picture(Source::File(path)).await,
            None => show_error(
                self,
                &gettext("Cannot Use Picture"),
                &gettext("Choose a file on a local file system."),
            ),
        }
    }

    pub(super) async fn paste_picture(&self) {
        let texture = self.clipboard().read_texture_future().await;
        match texture {
            Ok(Some(texture)) => {
                self.set_picture(Source::Png(texture.save_to_png_bytes()))
                    .await
            }
            _ => show_error(
                self,
                &gettext("Cannot Paste Picture"),
                &gettext("The clipboard holds no picture."),
            ),
        }
    }

    /// Makes the picture of the contact shown from `source`, in the
    /// background.
    pub(super) async fn set_picture(&self, source: Source) {
        let Some(slug) = self.imp().contact_view.slug() else {
            return;
        };
        // A copy for the thread writing.
        let vault = Vault::clone(&self.vault());
        let saved = gio::spawn_blocking(move || {
            let jpeg = jpeg(source).map_err(|err| err.message().to_owned())?;
            vault
                .save_contact_image(&slug, &jpeg)
                .map_err(|err| err.to_string())
        })
        .await
        .expect("making a picture does not panic");
        match saved {
            Ok(()) => self.reload(),
            Err(message) => show_error(self, &gettext("Cannot Use Picture"), &message),
        }
    }

    pub(super) async fn remove_picture(&self) {
        let Some(slug) = self.imp().contact_view.slug() else {
            return;
        };
        let path = Contact::image_path(self.vault().root(), &slug);
        if trash(self, &path, &gettext("Picture moved to trash")).await {
            self.reload();
        }
    }
}
