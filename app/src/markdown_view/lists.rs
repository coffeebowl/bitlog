//! Lists: each level is indented the same, tasks have check boxes, Tab
//! nests an item deeper or less deep, with all its lines, and Enter starts
//! the next item.

use std::collections::HashMap;
use std::ops::Range;

use bitlog_core::{
    Continuation, Formatting, MarkdownMode, TaskItem, continue_list, continue_quote,
    continue_table, nest_list_item, renumbered_lists,
};
use gtk::prelude::*;
use gtk::{glib, pango};

use super::check_boxes::CheckBox;
use super::decorations::Decorations;
use super::styling::Styling;
use super::{char_offsets, tags};

/// How much further each level of a list is indented than the one it is
/// in, whether it nests by two spaces, as bullets do, or three, as `1.`.
const LEVEL_INDENT: i32 = 24;
/// How wide bullets are shown with the spaces after them.
pub(super) const MARKER_WIDTH: i32 = 22;
/// Between a bullet and the text of its item.
pub(super) const MARKER_GAP: i32 = 6;

/// Indents the lists of `formatting` and gives its tasks check boxes,
/// which it adds to `decorations`.
pub(super) fn style(styling: &Styling, formatting: &Formatting, decorations: &mut Decorations) {
    indent(styling, formatting);
    style_tasks(styling, &formatting.tasks, decorations);
}

/// Indents the lines of list items by how deep their list is, by spacing
/// out the last of the spaces they are indented with, and widens bullets to
/// the same width, but for those check boxes take the place of. Numbers
/// stay as they are. Lines after the first of an item line up with its
/// text, and so do the lines they wrap into.
fn indent(styling: &Styling, formatting: &Formatting) {
    let text = styling.text;
    let bold = pango::AttrList::new();
    bold.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
    // Where the text of each line of a list item starts, by where the
    // line starts.
    let mut texts = HashMap::new();
    // How wide the markers of each item are, by where they start.
    let mut rooms = HashMap::new();
    for marker in &formatting.list_markers {
        if !is_bullet(text, marker) {
            let layout = styling.layout(&text[marker.clone()]);
            layout.set_attributes(Some(&bold));
            rooms.insert(marker.start, layout.pixel_size().0);
            continue;
        }
        rooms.insert(marker.start, MARKER_WIDTH);
        let is_check_box = formatting
            .tasks
            .iter()
            .any(|task| task.marker.start == marker.start);
        if !is_check_box && text[..marker.end].ends_with([' ', '\t']) {
            let pixels = MARKER_WIDTH - styling.width(&text[marker.clone()]);
            let space = styling.offset(marker.end) - 1;
            let spacing = tags::spacing(&styling.buffer, pixels);
            tags::apply(&styling.buffer, &spacing, space..space + 1);
        }
    }
    for indent in &formatting.list_indents {
        if indent.spaces.is_empty() {
            continue;
        }
        let marker = indent.marker.as_ref().map_or(0, |marker| {
            rooms.get(&marker.start).copied().unwrap_or(MARKER_WIDTH)
        });
        let target = i32::from(indent.depth.saturating_sub(1)) * LEVEL_INDENT + marker;
        let spaces = styling.chars(&indent.spaces);
        let last = spaces.end - 1;
        let pixels = target - styling.width(&text[indent.spaces.clone()]);
        let pixels = if pixels >= 0 && spaces.len() > 1 {
            pixels
        } else {
            // Wider than it should be, as spacing cannot be negative: only
            // the last space is left, which is then the first character of
            // its line, which Pango spaces out by half.
            tags::apply(&styling.buffer, tags::HIDDEN, spaces.start..last);
            2 * (target - styling.width(" "))
        };
        let spacing = tags::spacing(&styling.buffer, pixels);
        tags::apply(&styling.buffer, &spacing, last..last + 1);
        texts.insert(indent.spaces.start, target);
    }
    for marker in &formatting.list_markers {
        let line = text[..marker.start].rfind('\n').map_or(0, |at| at + 1);
        // Markers after others or after a quote marker keep their lines.
        if !text[line..marker.start].trim().is_empty() {
            continue;
        }
        let start = texts.get(&line).copied().unwrap_or(0);
        texts.insert(line, start + rooms[&marker.start]);
    }
    for (line, pixels) in texts {
        let hanging = tags::hanging(&styling.buffer, pixels);
        let line = styling.offset(line);
        tags::apply_to_lines(&styling.buffer, &hanging, line..line);
    }
}

/// Shows `tasks` with check boxes instead of their bullets and `[ ]`, but
/// for the one the cursor is at, and strikes out those that are done.
fn style_tasks(styling: &Styling, tasks: &[TaskItem], decorations: &mut Decorations) {
    for task in tasks {
        if task.done && !task.text.is_empty() {
            styling.tag(tags::TASK_DONE, &task.text);
        }
        decorations.revealable.push(styling.chars(&task.check_box));
        if styling.is_at(&task.check_box) {
            continue;
        }
        let toggle = if task.done { "[ ]" } else { "[x]" };
        let toggle = (task.check_box.clone(), toggle);
        // Bullets go, with their space: the check box takes their place, as
        // wide as markers are. Numbers stay.
        let (range, room) = if is_bullet(styling.text, &task.marker) {
            let marker = styling.offset(task.marker.start);
            decorations.bullets.retain(|(offset, _)| *offset != marker);
            (
                task.marker.start..task.check_box.end,
                MARKER_WIDTH - styling.width(" "),
            )
        } else {
            (task.check_box.clone(), 0)
        };
        let check_box = CheckBox::conceal(styling, &range, (task.done, room), toggle);
        decorations.check_boxes.push(check_box);
    }
}

/// Nests the list item at the cursor of `view` one level `deeper` or less
/// deep, and numbers the lists around it again. Whether the cursor is in a
/// list item, which then takes the key.
pub(super) fn nest(view: &gtk::TextView, mode: MarkdownMode, deeper: bool) -> bool {
    let buffer = view.buffer();
    // Tab indents several selected lines as they are, as in code.
    if let Some((start, end)) = buffer.selection_bounds()
        && start.line() != end.line()
    {
        return false;
    }
    let (text, at) = text_and_cursor(&buffer);
    let Some(edits) = nest_list_item(&text, mode, at, deeper) else {
        return false;
    };
    if edits.is_empty() {
        view.error_bell();
        return true;
    }
    buffer.begin_user_action();
    replace(&buffer, &text, &edits);
    renumber(&buffer, mode);
    buffer.end_user_action();
    true
}

/// Numbers the items of the lists at the cursor of `buffer` one after the
/// other again, as an item came, went or moved.
fn renumber(buffer: &gtk::TextBuffer, mode: MarkdownMode) {
    let (text, at) = text_and_cursor(buffer);
    replace(buffer, &text, &renumbered_lists(&text, mode, at));
}

/// Replaces the byte ranges of `text`, the text of `buffer`, given in
/// order.
fn replace(buffer: &gtk::TextBuffer, text: &str, edits: &[(Range<usize>, String)]) {
    let offsets = char_offsets(text);
    // From the back, so that earlier offsets stay valid.
    for (range, replacement) in edits.iter().rev() {
        let mut start = buffer.iter_at_offset(offsets[range.start]);
        let mut end = buffer.iter_at_offset(offsets[range.end]);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, replacement);
    }
}

/// Whether `marker` is a bullet, not a number.
fn is_bullet(text: &str, marker: &Range<usize>) -> bool {
    matches!(text[marker.clone()].trim_end(), "-" | "*" | "+")
}

/// Starts the next row of a table, list item or line of a quote after the
/// one at the cursor of `view`, or ends the table, list or quote if the
/// row, item or line is empty. Numbered items after a new one count on.
/// Whether the cursor is in a table, or after the marker of a list item or
/// quote, which then takes the key.
pub(super) fn continue_item(view: &gtk::TextView, mode: MarkdownMode) -> bool {
    let buffer = view.buffer();
    if buffer.has_selection() {
        return false;
    }
    let (text, at) = text_and_cursor(&buffer);
    let continuation = continue_table(&text, mode, at)
        .or_else(|| continue_list(&text, mode, at))
        .or_else(|| continue_quote(&text, mode, at));
    let Some(continuation) = continuation else {
        return false;
    };
    let offsets = char_offsets(&text);
    buffer.begin_user_action();
    match continuation {
        Continuation::Insert(next) => {
            buffer.insert_at_cursor(&next);
            renumber(&buffer, mode);
        }
        Continuation::End(marker) => {
            let mut start = buffer.iter_at_offset(offsets[marker.start]);
            let mut end = buffer.iter_at_offset(offsets[marker.end]);
            buffer.delete(&mut start, &mut end);
        }
        Continuation::Outdent => {
            nest(view, mode, false);
        }
        Continuation::Replace {
            range,
            before,
            after,
        } => {
            let mut start = buffer.iter_at_offset(offsets[range.start]);
            let mut end = buffer.iter_at_offset(offsets[range.end]);
            buffer.delete(&mut start, &mut end);
            let cursor = start.offset() + before.chars().count() as i32;
            buffer.insert(&mut start, &format!("{before}{after}"));
            buffer.place_cursor(&buffer.iter_at_offset(cursor));
        }
    }
    buffer.end_user_action();
    view.scroll_mark_onscreen(&buffer.get_insert());
    true
}

/// The text of `buffer` and the byte offset of its cursor.
pub(super) fn text_and_cursor(buffer: &gtk::TextBuffer) -> (glib::GString, usize) {
    let (start, end) = buffer.bounds();
    let text = buffer.text(&start, &end, true);
    let cursor = buffer.iter_at_mark(&buffer.get_insert()).offset();
    let at = usize::try_from(cursor)
        .ok()
        .and_then(|cursor| text.char_indices().nth(cursor))
        .map_or(text.len(), |(byte, _)| byte);
    (text, at)
}
