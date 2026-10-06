//! Adding images: pasted or dropped onto the view, they are put into the
//! images folder of the vault and linked at the cursor, or where they were
//! dropped. Only while the text is editable and the view knows where it is
//! saved.

use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{add_image, add_pasted_image, image_link};
use chrono::Local;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use super::MarkdownView;
use crate::alert::show_error;

/// An image to add.
enum Source {
    File(PathBuf),
    /// Pasted pixels, without a file, like a screenshot.
    Pixels(gdk::Texture),
}

impl MarkdownView {
    /// Adds the images of the clipboard when pasting: image files copied in
    /// a file manager, or an image without text. Apps that copy text along
    /// with a picture of it, like spreadsheets, paste their text. Before
    /// `link_pasted_addresses`, which this stops for images.
    pub(super) fn paste_images(&self) {
        self.connect_paste_clipboard(|view| {
            if !view.takes_images() {
                return;
            }
            let clipboard = view.clipboard();
            let formats = clipboard.formats().union_deserialize_types();
            let has_files = formats.contains_type(gdk::FileList::static_type());
            let has_pixels = formats.contains_type(gdk::Texture::static_type())
                && !formats.contains_type(glib::Type::STRING);
            if !has_files && !has_pixels {
                return;
            }
            view.stop_signal_emission_by_name("paste-clipboard");
            glib::spawn_future_local(glib::clone!(
                #[weak]
                view,
                async move {
                    let sources = if has_files {
                        clipboard
                            .read_value_future(
                                gdk::FileList::static_type(),
                                glib::Priority::DEFAULT,
                            )
                            .await
                            .ok()
                            .and_then(|value| value.get::<gdk::FileList>().ok())
                            .map(|files| image_files(&files))
                            .unwrap_or_default()
                    } else {
                        clipboard
                            .read_texture_future()
                            .await
                            .ok()
                            .flatten()
                            .map(|texture| vec![Source::Pixels(texture)])
                            .unwrap_or_default()
                    };
                    if sources.is_empty() {
                        // Other files paste their paths.
                        view.buffer()
                            .paste_clipboard(&clipboard, None, view.is_editable());
                    } else {
                        view.add_images(sources).await;
                    }
                }
            ));
        });
    }

    /// Adds the image files dropped onto the view where they were dropped.
    /// Before the view itself, which would take them for text.
    pub(super) fn add_dropped_images(&self) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        target.set_propagation_phase(gtk::PropagationPhase::Capture);
        target.connect_accept(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            false,
            move |_, _| view.takes_images()
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            false,
            move |_, value, x, y| {
                let Ok(files) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let sources = image_files(&files);
                if sources.is_empty() || !view.takes_images() {
                    return false;
                }
                view.grab_focus();
                if let Some(iter) = view.iter_at(x, y) {
                    view.buffer().place_cursor(&iter);
                }
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    view,
                    async move { view.add_images(sources).await }
                ));
                true
            }
        ));
        self.add_controller(target);
    }

    fn takes_images(&self) -> bool {
        self.is_editable() && self.imp().location.borrow().is_some()
    }

    /// Puts the images of `sources` into the images folder in the
    /// background, then links them in place of the selection, or at the
    /// cursor, each on a line of its own: images are drawn below their
    /// line, so text beside them would read out of order.
    async fn add_images(&self, sources: Vec<Source>) {
        let Some(location) = self.imp().location.borrow().clone() else {
            return;
        };
        let root = location.root.clone();
        let added = gio::spawn_blocking(move || {
            sources
                .into_iter()
                .map(|source| match source {
                    Source::File(path) => add_image(&root, &path),
                    Source::Pixels(texture) => add_pasted_image(
                        &root,
                        &texture.save_to_png_bytes(),
                        Local::now().naive_local(),
                    ),
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .expect("adding images does not panic");
        let images = match added {
            Ok(images) => images,
            Err(err) => {
                show_error(self, &gettext("Cannot Add Image"), &err.to_string());
                return;
            }
        };
        // The text may have gone elsewhere meanwhile.
        if !self.is_editable() || self.imp().location.borrow().as_ref() != Some(&location) {
            return;
        }
        let mut links: Vec<String> = images
            .iter()
            .map(|image| image_link(&location.file, image))
            .collect();
        let buffer = self.buffer();
        buffer.begin_user_action();
        buffer.delete_selection(true, true);
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let text_after = !cursor.ends_line();
        if !cursor.starts_line() {
            links.insert(0, String::new());
        }
        if text_after {
            links.push(String::new());
        }
        buffer.insert_at_cursor(&links.join("\n"));
        if text_after {
            // Before the text that was after the cursor, not after it.
            let mut end = buffer.iter_at_mark(&buffer.get_insert());
            end.backward_char();
            buffer.place_cursor(&end);
        }
        buffer.end_user_action();
        self.scroll_mark_onscreen(&buffer.get_insert());
    }
}

/// The image files of `files`, as their names tell.
fn image_files(files: &gdk::FileList) -> Vec<Source> {
    files
        .files()
        .iter()
        .filter_map(|file| file.path())
        .filter(|path| {
            let (content_type, _) = gio::content_type_guess(Some(path), None);
            gio::content_type_is_mime_type(&content_type, "image/*")
        })
        .map(Source::File)
        .collect()
}
