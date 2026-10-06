//! A note as the editor shows it while typing: formatted, with the markers
//! of the bold word visible only because the cursor is in it.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{glib, graphene, pango};

use super::{append_check_box, append_layout, markup_layout};
use crate::colors::with_alpha;

const LINE_HEIGHT: f32 = 28.0;
const HEADING_HEIGHT: f32 = 36.0;
/// Where list items start, after their bullet or check box.
const ITEM_X: f32 = 26.0;
pub(super) const HEIGHT: f32 = HEADING_HEIGHT + 4.0 * LINE_HEIGHT;
/// As the editor dims Markdown.
const MARKUP_ALPHA: &str = "45%";

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let escape = |text: &str| glib::markup_escape_text(text).to_string();

    let heading = format!(
        "<span size=\"x-large\" weight=\"bold\">{}</span>",
        escape(&gettext("Release checklist"))
    );
    let heading = markup_layout(widget, &heading, Some(width));
    append_layout(
        snapshot,
        &heading,
        0.0,
        HEADING_HEIGHT / 2.0,
        0.0,
        &foreground,
    );

    // The sentence with its bold word between markers, typed with the
    // cursor at the end of that word.
    // Translators: Keep the two ** around the word shown in bold.
    let sentence = gettext("Deploy **after** the evening traffic drops.");
    let parts: Vec<&str> = sentence.splitn(3, "**").collect();
    let (before, bold, after) = match parts[..] {
        [before, bold, after] => (before, bold, after),
        _ => (sentence.as_str(), "", ""),
    };
    let marker = format!("<span alpha=\"{MARKUP_ALPHA}\">**</span>");
    let markup = format!(
        "{}{marker}<b>{}</b>{marker}{}",
        escape(before),
        escape(bold),
        escape(after)
    );
    let line = markup_layout(widget, &markup, Some(width));
    let y = HEADING_HEIGHT + LINE_HEIGHT / 2.0;
    append_layout(snapshot, &line, 0.0, y, 0.0, &foreground);
    let cursor = (before.len() + 2 + bold.len()) as i32;
    let (strong, _) = line.cursor_pos(cursor);
    let (_, line_height) = line.pixel_size();
    let x = strong.x() as f32 / pango::SCALE as f32;
    let cursor = graphene::Rect::new(x, y - line_height as f32 / 2.0, 1.5, line_height as f32);
    snapshot.append_color(&foreground, &cursor);

    let items = [
        (Item::Bullet, gettext("Tag the release")),
        (Item::Open, gettext("Run the pipeline")),
        (Item::Done, gettext("Check the dashboards")),
    ];
    for (index, (item, text)) in items.iter().enumerate() {
        let y = HEADING_HEIGHT + (index as f32 + 1.5) * LINE_HEIGHT;
        let room = Some(width - ITEM_X);
        let (line, color) = match item {
            Item::Bullet => {
                let dot = graphene::Rect::new(6.0, y - 3.0, 6.0, 6.0);
                super::fill_rounded(snapshot, dot, 3.0, &with_alpha(&foreground, 0.7));
                (markup_layout(widget, &escape(text), room), foreground)
            }
            Item::Open => {
                append_check_box(snapshot, 1.0, y, false, &foreground);
                (markup_layout(widget, &escape(text), room), foreground)
            }
            Item::Done => {
                append_check_box(snapshot, 1.0, y, true, &foreground);
                let markup = format!("<s>{}</s>", escape(text));
                (
                    markup_layout(widget, &markup, room),
                    with_alpha(&foreground, 0.55),
                )
            }
        };
        append_layout(snapshot, &line, ITEM_X, y, 0.0, &color);
    }
}

enum Item {
    Bullet,
    Open,
    Done,
}
