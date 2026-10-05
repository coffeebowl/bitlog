//! Editing helpers: check boxes, list items, brackets, markers around the
//! selection and tidying up tables.

use std::ops::Range;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::pairs_bracket;
use gtk::{gdk, glib};

use super::check_boxes::CheckBox;
use super::tables::TidiedTable;
use super::{MarkdownView, lists};

impl MarkdownView {
    /// What a click on the check box at `x`, `y` in widget coordinates
    /// replaces, and with what, if there is one.
    pub(super) fn check_box_at(&self, x: f64, y: f64) -> Option<(Range<i32>, String)> {
        self.restyle_if_queued();
        let (x, y) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let decorations = self.imp().decorations.borrow();
        decorations
            .check_box_at(self.upcast_ref(), x, y)
            .map(CheckBox::toggle)
    }

    /// A click on a check box checks or unchecks it, before the view would
    /// move the cursor there.
    pub(super) fn toggle_check_boxes_on_click(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |click, _, x, y| {
                if !view.is_editable() {
                    return;
                }
                let Some((range, replacement)) = view.check_box_at(x, y) else {
                    return;
                };
                click.set_state(gtk::EventSequenceState::Claimed);
                let buffer = view.buffer();
                let mut start = buffer.iter_at_offset(range.start);
                let mut end = buffer.iter_at_offset(range.end);
                buffer.begin_user_action();
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &replacement);
                buffer.end_user_action();
            }
        ));
        self.add_controller(click);
    }

    /// Tidies up a table the cursor left, when idle, as the text may be
    /// changing, unless the table changed since.
    pub(super) fn queue_tidy(&self, tidied: TidiedTable) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || {
                let buffer = view.buffer();
                let (table, text) = &tidied.table;
                let mut start = buffer.iter_at_offset(table.start);
                let mut end = buffer.iter_at_offset(table.end);
                if !view.is_editable() || buffer.text(&start, &end, true) != text.as_str() {
                    return;
                }
                buffer.begin_user_action();
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &tidied.tidied);
                buffer.end_user_action();
            }
        ));
    }

    /// In list items, Tab nests the item deeper, Shift+Tab less deep, and
    /// Enter starts the next item, as it starts the next row in tables,
    /// before the view would handle the keys. Shift+Enter still only
    /// breaks the line. Brackets come in pairs, so that wiki links are
    /// quick to type.
    pub(super) fn edit_by_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let others = gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK;
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                if !view.is_editable() || modifiers.intersects(others) {
                    return glib::Propagation::Proceed;
                }
                let is_handled = match key {
                    gdk::Key::Tab => lists::nest(view.upcast_ref(), view.mode(), !shift),
                    gdk::Key::ISO_Left_Tab => lists::nest(view.upcast_ref(), view.mode(), false),
                    gdk::Key::Return | gdk::Key::KP_Enter if !shift => {
                        lists::continue_item(view.upcast_ref(), view.mode())
                    }
                    gdk::Key::bracketleft => view.type_bracket(),
                    gdk::Key::bracketright => view.skip_bracket(),
                    gdk::Key::BackSpace => view.delete_brackets(),
                    _ => false,
                };
                if is_handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
    }

    /// Types `[` with the `]` closing it, where `pairs_bracket` says so, or
    /// puts the selection in brackets. Whether that took the key.
    fn type_bracket(&self) -> bool {
        let buffer = self.buffer();
        if let Some((start, end)) = buffer.selection_bounds() {
            let (start, end) = (start.offset(), end.offset());
            buffer.begin_user_action();
            buffer.insert(&mut buffer.iter_at_offset(end), "]");
            buffer.insert(&mut buffer.iter_at_offset(start), "[");
            buffer.select_range(
                &buffer.iter_at_offset(start + 1),
                &buffer.iter_at_offset(end + 1),
            );
            buffer.end_user_action();
            return true;
        }
        let (text, at) = lists::text_and_cursor(&buffer);
        if !pairs_bracket(&text, at) {
            return false;
        }
        buffer.begin_user_action();
        buffer.insert_at_cursor("[]");
        let mut cursor = buffer.iter_at_mark(&buffer.get_insert());
        cursor.backward_char();
        buffer.place_cursor(&cursor);
        buffer.end_user_action();
        true
    }

    /// Steps over the `]` at the cursor instead of typing another one, as
    /// it may have come with its `[`. Whether there is one.
    fn skip_bracket(&self) -> bool {
        let buffer = self.buffer();
        let mut cursor = buffer.iter_at_mark(&buffer.get_insert());
        if buffer.has_selection() || cursor.char() != ']' {
            return false;
        }
        cursor.forward_char();
        buffer.place_cursor(&cursor);
        true
    }

    /// Takes away both brackets of an empty pair at the cursor, as they
    /// may have been typed together. Whether there is one.
    fn delete_brackets(&self) -> bool {
        let buffer = self.buffer();
        let mut end = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = end;
        if buffer.has_selection()
            || end.char() != ']'
            || !start.backward_char()
            || start.char() != '['
        {
            return false;
        }
        end.forward_char();
        buffer.begin_user_action();
        buffer.delete(&mut start, &mut end);
        buffer.end_user_action();
        true
    }

    /// Puts `marker` around the selection, or at the cursor, and selects
    /// what it wraps. Takes it away instead if it is there already.
    pub(super) fn toggle_marker(&self, marker: &str) {
        let buffer = self.buffer();
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            (cursor, cursor)
        });
        let (start, end) = (start.offset(), end.offset());
        let length = i32::try_from(marker.chars().count()).expect("markers are short");
        let mark = marker.chars().next().expect("markers are not empty");
        let is_mark = |iter: &gtk::TextIter| iter.char() == mark;
        // How many marker characters there are on each side, as in `***`.
        let mut before = 0;
        let mut iter = buffer.iter_at_offset(start);
        while iter.backward_char() && is_mark(&iter) {
            before += 1;
        }
        let mut after = 0;
        let mut iter = buffer.iter_at_offset(end);
        // The end of the text has no character, so the loop stops there.
        while is_mark(&iter) {
            after += 1;
            iter.forward_char();
        }
        let run = before.min(after);
        let is_wrapped = match marker {
            // In `***both***`, one star is italic and two are bold.
            "*" => run % 2 == 1,
            "**" => run >= 2,
            _ => run >= 1,
        };

        buffer.begin_user_action();
        let (start, end) = if is_wrapped {
            buffer.delete(
                &mut buffer.iter_at_offset(end),
                &mut buffer.iter_at_offset(end + length),
            );
            buffer.delete(
                &mut buffer.iter_at_offset(start - length),
                &mut buffer.iter_at_offset(start),
            );
            (start - length, end - length)
        } else {
            buffer.insert(&mut buffer.iter_at_offset(end), marker);
            buffer.insert(&mut buffer.iter_at_offset(start), marker);
            (start + length, end + length)
        };
        buffer.select_range(&buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
        buffer.end_user_action();
    }
}
