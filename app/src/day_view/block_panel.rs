//! The panel of the day view that shows the block chosen: its title,
//! time, project and text, and how wide it is.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{BlockId, RemovedText, minute_of_day, time_at_minute};
use chrono::NaiveTime;
use gettextrs::gettext;
use gtk::{gio, glib};

use super::DayView;
use crate::config;
use crate::format::{format_span, format_time};
use crate::note_view::existing_notes;
use crate::project_picker::{project_markup, project_popover};

impl DayView {
    /// Makes the block panel as wide as dragged at its edge, and remembers
    /// that width.
    pub(super) fn setup_panel_resizing(&self) {
        let imp = self.imp();
        let settings = gio::Settings::new(config::app_id());
        imp.split_view
            .set_max_sidebar_width(settings.int("block-panel-width").into());
        imp.panel_handle.set_cursor_from_name(Some("ew-resize"));

        // The view takes the drag, as the pointer soon leaves the thin edge.
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |drag, x, y| {
                let imp = view.imp();
                let at_edge = imp.split_view.shows_sidebar()
                    && imp
                        .split_view
                        .compute_point(
                            &*imp.panel_handle,
                            &gtk::graphene::Point::new(x as f32, y as f32),
                        )
                        .is_some_and(|point| {
                            imp.panel_handle
                                .contains(point.x().into(), point.y().into())
                        });
                drag.set_state(if at_edge {
                    gtk::EventSequenceState::Claimed
                } else {
                    gtk::EventSequenceState::Denied
                });
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |drag, offset, _| {
                let split_view = &view.imp().split_view;
                let Some((start, _)) = drag.start_point() else {
                    return;
                };
                // The panel is at the end, so it grows towards the start.
                let total = f64::from(split_view.width());
                let width = match view.direction() {
                    gtk::TextDirection::Rtl => start + offset,
                    _ => total - start - offset,
                };
                let width = width
                    .min(total * split_view.sidebar_width_fraction())
                    .max(split_view.min_sidebar_width());
                split_view.set_max_sidebar_width(width.round());
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _, _| {
                let width = view.imp().split_view.max_sidebar_width();
                if let Err(err) = settings.set_int("block-panel-width", width as i32) {
                    glib::g_warning!("bitlog", "Cannot save the width of the block panel: {err}");
                }
            }
        ));
        imp.split_view.add_controller(drag);
    }

    /// Deletes the block shown, asking what happens to its text.
    pub(super) async fn delete_block(&self) {
        let imp = self.imp();
        let id = imp
            .shown_block
            .borrow()
            .clone()
            .expect("the delete button belongs to the block shown");
        let has_text = imp
            .file
            .borrow()
            .as_ref()
            .and_then(|file| file.day.block(&id))
            .is_some_and(|block| !block.text.is_empty());
        if !has_text {
            self.update(|day| day.remove_block(&id, RemovedText::Discard));
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Delete Block?")),
            Some(&gettext(
                "The text of the block can be moved to the end of the day note",
            )),
        );
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("discard", &gettext("_Delete Text")),
            ("move", &gettext("_Move to Note")),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("move"));
        dialog.set_close_response("cancel");
        let text = match dialog.choose_future(Some(self)).await.as_str() {
            "discard" => RemovedText::Discard,
            "move" => RemovedText::MoveToNote,
            _ => return,
        };
        self.update(|day| day.remove_block(&id, text));
    }

    pub(super) fn block_id(&self, index: usize) -> BlockId {
        let file = self.imp().file.borrow();
        let day = &file
            .as_ref()
            .expect("blocks are only shown with their day")
            .day;
        day.blocks[index].id.clone()
    }

    /// Shows the block `id` in the panel, or closes the panel if the day
    /// has no such block.
    pub fn show_block_by_id(&self, id: &BlockId) {
        let imp = self.imp();
        let index = imp
            .file
            .borrow()
            .as_ref()
            .and_then(|file| file.day.blocks.iter().position(|block| block.id == *id));
        match index {
            Some(index) => self.show_block(index),
            None => imp.split_view.set_show_sidebar(false),
        }
    }

    /// Lets the spin buttons of block times show and take times, and fills
    /// them when their popover opens.
    pub(super) fn setup_time_popovers(&self) {
        let imp = self.imp();
        for spin in [&imp.block_start, &imp.block_end] {
            spin.connect_output(|spin| {
                spin.set_text(&format_time(spin_time(spin)));
                glib::Propagation::Stop
            });
            spin.connect_input(|spin| {
                Some(
                    NaiveTime::parse_from_str(spin.text().trim(), "%H:%M")
                        .map(|time| f64::from(minute_of_day(time)))
                        .map_err(|_| ()),
                )
            });
        }
        imp.block_time_popover.connect_show(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| {
                let imp = view.imp();
                let id = imp.shown_block.borrow().clone();
                let file = imp.file.borrow();
                let day = &file.as_ref().expect("the popover belongs to a day").day;
                let block = id
                    .and_then(|id| day.block(&id))
                    .expect("the popover belongs to the block shown");
                set_spin_time(&imp.block_start, block.start);
                set_spin_time(&imp.block_end, block.end);
            }
        ));
    }

    /// Shows title, time, project and text of the block `index` in the panel.
    pub(super) fn show_block(&self, index: usize) {
        let imp = self.imp();
        // Saving texts does not reorder the blocks, so `index` stays valid.
        self.save_texts_now();
        imp.timeline.select(Some(index));
        let vault = self.vault();
        let file = imp.file.borrow();
        let block = &file
            .as_ref()
            .expect("blocks are only shown with their day")
            .day
            .blocks[index];
        let is_other = imp.shown_block.borrow().as_ref() != Some(&block.id);
        imp.shown_block.replace(Some(block.id.clone()));
        let project = vault.project(&block.project);
        let project_name = vault.project_name(&block.project);
        // Keeps what is being typed into the same block, see `show_details`.
        if is_other || imp.block_title.text() != block.title {
            imp.block_title.set_text(&block.title);
        }
        imp.block_title.set_placeholder_text(Some(project_name));
        imp.block_time_button
            .set_label(&format_span(block.start, block.end));
        imp.block_project_label.set_label(&project.map_or_else(
            || glib::markup_escape_text(project_name).to_string(),
            project_markup,
        ));
        let id = block.id.clone();
        imp.block_project_button.set_popover(Some(&project_popover(
            &vault,
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |project| {
                    let vault = view.vault();
                    view.update(|day| day.set_block_project(&id, project, vault.projects()));
                }
            ),
        )));
        imp.block_text
            .set_wiki_links(Some(block.project.clone()), existing_notes(&vault));
        if is_other || !imp.block_text.shows(&block.text) {
            imp.block_text.set_markdown(&block.text);
        }
        imp.split_view.set_show_sidebar(true);
    }
}

/// The time of `spin`, whose value counts the minutes of the day.
pub(super) fn spin_time(spin: &gtk::SpinButton) -> NaiveTime {
    time_at_minute(spin.value() as u32)
}

fn set_spin_time(spin: &gtk::SpinButton, time: NaiveTime) {
    spin.set_value(f64::from(minute_of_day(time)));
}
