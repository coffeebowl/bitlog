//! The search with the start of a project name typed: a command to add a
//! block for the project, the project and a block that mentions it.

use chrono::{Days, Local};
use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{glib, graphene, gsk};

use super::{append_layout, fill_rounded, layout, markup_layout};
use crate::colors::with_alpha;
use crate::drawing;
use crate::format::format_full_date;

const ENTRY_HEIGHT: f32 = 52.0;
const ROW_HEIGHT: f32 = 48.0;
const ROWS: usize = 3;
const INSET: f32 = 14.0;
pub(super) const HEIGHT: f32 = ENTRY_HEIGHT + ROW_HEIGHT * ROWS as f32;

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let card = graphene::Rect::new(0.0, 0.0, width, HEIGHT);
    fill_rounded(snapshot, card, 12.0, &with_alpha(&foreground, 0.03));
    let edge = with_alpha(&foreground, 0.12);
    let rounded = gsk::RoundedRect::from_rect(card, 12.0);
    snapshot.append_border(&rounded, &[1.0; 4], &[edge; 4]);

    let project = gettext("Webshop");
    let typed = project.chars().take(4).collect::<String>().to_lowercase();
    let field = graphene::Rect::new(10.0, 10.0, width - 20.0, ENTRY_HEIGHT - 20.0);
    fill_rounded(snapshot, field, 8.0, &with_alpha(&foreground, 0.08));
    let middle = ENTRY_HEIGHT / 2.0;
    let text = layout(widget, &typed, None);
    append_layout(
        snapshot,
        &text,
        (INSET + 8.0, middle),
        (0.0, 0.5),
        &foreground,
    );
    let cursor_x = INSET + 9.0 + text.pixel_size().0 as f32;
    let cursor = graphene::Rect::new(cursor_x, middle - 9.0, 1.0, 18.0);
    snapshot.append_color(&foreground, &cursor);
    snapshot.append_color(&edge, &graphene::Rect::new(0.0, ENTRY_HEIGHT, width, 1.0));

    let yesterday = Local::now().date_naive() - Days::new(1);
    let block = gettext("Checkout bug in the webshop");
    let rows = [
        (
            glib::markup_escape_text(
                // Translators: A command of the search, as in "New Block for Webshop".
                &gettext("New Block for {project}").replace("{project}", &project),
            )
            .to_string(),
            None,
        ),
        (
            glib::markup_escape_text(&project).to_string(),
            Some(gettext("Project")),
        ),
        (
            bold_match(&block, &typed),
            // Translators: Where a search found something, as in "Block · September 22, 2026".
            Some(gettext("Block · {date}").replace("{date}", &format_full_date(yesterday))),
        ),
    ];
    let dim = with_alpha(&foreground, 0.55);
    let room = Some(width - 2.0 * INSET);
    for (index, (title, place)) in rows.iter().enumerate() {
        let top = ENTRY_HEIGHT + index as f32 * ROW_HEIGHT;
        let middle = top + ROW_HEIGHT / 2.0;
        // The first row is selected, as Enter runs it.
        if index == 0 {
            let selected = graphene::Rect::new(6.0, top + 4.0, width - 12.0, ROW_HEIGHT - 8.0);
            fill_rounded(snapshot, selected, 6.0, &with_alpha(&foreground, 0.1));
        }
        let title = markup_layout(widget, title, room);
        let Some(place) = place else {
            append_layout(snapshot, &title, (INSET, middle), (0.0, 0.5), &foreground);
            continue;
        };
        append_layout(snapshot, &title, (INSET, middle), (0.0, 1.0), &foreground);
        let place = drawing::layout(widget, place, 0.85, false);
        append_layout(snapshot, &place, (INSET, middle + 1.0), (0.0, 0.0), &dim);
    }
}

/// `text` in Pango markup, with the first match of `typed` in bold, the
/// case aside, as the search shows what it found.
fn bold_match(text: &str, typed: &str) -> String {
    let lower = text.to_lowercase();
    let found = lower.find(typed).filter(|&start| {
        lower.len() == text.len()
            && text.is_char_boundary(start)
            && text.is_char_boundary(start + typed.len())
    });
    let Some(start) = found else {
        return glib::markup_escape_text(text).to_string();
    };
    let end = start + typed.len();
    format!(
        "{}<b>{}</b>{}",
        glib::markup_escape_text(&text[..start]),
        glib::markup_escape_text(&text[start..end]),
        glib::markup_escape_text(&text[end..])
    )
}
