//! Following links: wiki links to notes of the vault, and web links that
//! other apps open.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::NotePath;
use gtk::{gdk, gio, glib};

use super::MarkdownView;

/// Where a link leads.
#[derive(Debug, Clone)]
pub(super) enum Target {
    /// A note of the vault, or none if the wiki link points nowhere.
    Note(Option<NotePath>),
    /// A web page or mail address, which another app opens.
    Web(String),
}

impl MarkdownView {
    /// Where the link at `iter` leads, if there is one.
    fn link_at(&self, iter: &gtk::TextIter) -> Option<Target> {
        self.restyle_if_queued();
        self.imp()
            .links
            .borrow()
            .iter()
            .find(|(range, _)| range.contains(&iter.offset()))
            .map(|(_, target)| target.clone())
    }

    /// Follows the link at `iter`, if there is one.
    pub(super) fn follow_link_at(&self, iter: &gtk::TextIter) {
        if let Some(target) = self.link_at(iter) {
            self.follow_link(target);
        }
    }

    /// Opens a note of the vault, or another app for a web page or mail
    /// address.
    fn follow_link(&self, target: Target) {
        match target {
            Target::Note(Some(note)) => {
                self.emit_by_name::<()>("wiki-link-activated", &[&note.to_string()]);
            }
            // Points nowhere, like `[[a/b/c]]`.
            Target::Note(None) => self.error_bell(),
            Target::Web(url) => {
                let window = self.root().and_downcast::<gtk::Window>();
                gtk::UriLauncher::new(&url).launch(
                    window.as_ref(),
                    None::<&gio::Cancellable>,
                    |_| {},
                );
            }
        }
    }

    /// The text at `x`, `y` in widget coordinates, if there is any.
    fn iter_at(&self, x: f64, y: f64) -> Option<gtk::TextIter> {
        let (x, y) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        // Below the text of a line, GTK takes the pointer for the end of the
        // line, counting its hidden text as shown, and aborts if it has any.
        // Lines are higher than their text by the space below them, and for
        // a moment after their text shrank, until they are laid out again.
        let (line, _) = self.line_at_y(y);
        let mut end = line;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let (last_row, _) = self.cursor_locations(Some(&end));
        if y >= last_row.y() + last_row.height() {
            return None;
        }
        self.iter_at_location(x, y)
    }

    /// A click on a link follows it, as in the "Hypertext" demo of GTK, and
    /// the pointer shows where that is possible.
    pub(super) fn follow_links_on_click(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        // The link under the pointer as the button goes down: the text may
        // move before it goes up, as the cursor reveals the markup.
        let pressed = Rc::new(RefCell::new(None));
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[strong]
            pressed,
            move |_, _, x, y| {
                let link = view.iter_at(x, y).and_then(|iter| view.link_at(&iter));
                pressed.replace(link);
            }
        ));
        click.connect_released(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, presses, _, _| {
                let link = pressed.take();
                // Selecting text is no click on a link.
                if presses != 1 || view.buffer().has_selection() {
                    return;
                }
                if let Some(target) = link {
                    view.follow_link(target);
                }
            }
        ));
        self.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, x, y| {
                let on_link = view
                    .iter_at(x, y)
                    .is_some_and(|iter| view.link_at(&iter).is_some());
                let on_check_box = view.is_editable() && view.check_box_at(x, y).is_some();
                let pointer = on_link || on_check_box;
                view.set_cursor_from_name(Some(if pointer { "pointer" } else { "text" }));
            }
        ));
        self.add_controller(motion);
    }
}
