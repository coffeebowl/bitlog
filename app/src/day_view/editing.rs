//! Typing into the day note and the block shown, and saving what is typed.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::Block;
use gtk::glib;

use super::DayView;
use crate::markdown_view::MarkdownView;

impl DayView {
    /// Saves the texts a second after the last change, or when they are left.
    pub(super) fn setup_text_editing(&self) {
        let imp = self.imp();
        for view in [&*imp.note_view, &*imp.block_text] {
            view.connect_edited(glib::clone!(
                #[weak(rename_to = day_view)]
                self,
                move |_| day_view.save_texts_later()
            ));
        }
        imp.block_title.connect_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |entry| {
                // Filling in the title of the block shown is no edit.
                if view
                    .shown_block()
                    .is_some_and(|block| block.title != entry.text())
                {
                    view.save_texts_later();
                }
            }
        ));
        // Enter finishes the title, leaving it saves it.
        imp.block_title.connect_activate(|entry| {
            if let Some(root) = entry.root() {
                root.set_focus(None::<&gtk::Widget>);
            }
        });
        let editors: [&gtk::Widget; 3] = [
            imp.note_view.upcast_ref(),
            imp.block_text.upcast_ref(),
            imp.block_title.upcast_ref(),
        ];
        for editor in editors {
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.save_texts_now()
            ));
            editor.add_controller(focus);
        }
        // A title left with its text selected would stay grey behind it.
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = entry)]
            imp.block_title,
            move |_| entry.select_region(0, 0)
        ));
        imp.block_title.add_controller(focus);

        // Clicking beside the editor being typed into leaves it, even where
        // nothing takes the focus, like the background or a label.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |click, _, x, y| {
                // Clicks into popovers arrive here too, with coordinates
                // that do not belong to this page.
                let surface = click.current_event().and_then(|event| event.surface());
                if view.native().and_then(|native| native.surface()) != surface {
                    return;
                }
                let Some(root) = view.root() else { return };
                let Some(focus) = root.focus() else { return };
                let imp = view.imp();
                let editors: [&gtk::Widget; 3] = [
                    imp.note_view.upcast_ref(),
                    imp.block_text.upcast_ref(),
                    imp.block_title.upcast_ref(),
                ];
                let Some(editor) = editors
                    .into_iter()
                    .find(|editor| focus == **editor || focus.is_ancestor(*editor))
                else {
                    return;
                };
                let inside = view
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .is_some_and(|picked| picked == *editor || picked.is_ancestor(editor));
                if !inside {
                    root.set_focus(None::<&gtk::Widget>);
                }
            }
        ));
        self.add_controller(click);
    }

    pub(super) fn save_texts_later(&self) {
        self.imp().text_save.schedule(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.save_texts()
        ));
    }

    /// Saves the texts being typed, if there are unsaved changes.
    pub fn save_texts_now(&self) {
        if self.imp().text_save.cancel() {
            self.save_texts();
        }
    }

    /// Saves day note, title and text of the block shown as they are typed.
    /// Texts that were not edited are left alone, with any headings written
    /// outside the app.
    ///
    /// On failure the typed texts stay, so that nothing gets lost.
    pub(super) fn save_texts(&self) {
        let imp = self.imp();
        let Some(note) = imp.file.borrow().as_ref().map(|file| file.day.note.clone()) else {
            return;
        };
        let edited =
            |view: &MarkdownView, saved: &str| (!view.shows(saved)).then(|| view.markdown());
        let note = edited(&imp.note_view, &note);
        let block = self.shown_block().map(|block| {
            let title = imp.block_title.text().to_string();
            (block.id, title, edited(&imp.block_text, &block.text))
        });
        let result = self.save(|day| {
            if let Some(note) = note {
                day.note = note;
            }
            if let Some((id, title, text)) = block {
                day.set_block_title(&id, &title)?;
                if let Some(text) = text {
                    day.set_block_text(&id, &text)?;
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            self.show_save_error(&err);
        }
    }

    /// The block shown in the panel, as saved.
    pub(super) fn shown_block(&self) -> Option<Block> {
        let imp = self.imp();
        let id = imp.shown_block.borrow().clone()?;
        let file = imp.file.borrow();
        file.as_ref()?.day.block(&id).cloned()
    }
}
