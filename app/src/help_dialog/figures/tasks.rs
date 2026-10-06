//! A short task list as a card of rows: open tasks, one of them overdue,
//! and a finished one.

use chrono::{Days, Local};
use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use super::{append_check_box, append_layout, layout, markup_layout};
use crate::colors::{parse, with_alpha};
use crate::format::format_short_date;

const ROW_HEIGHT: f32 = 44.0;
const ROWS: usize = 4;
const INSET: f32 = 14.0;
/// Where the titles start, after the check boxes.
const TITLE_X: f32 = 44.0;
/// Room for the due dates.
const DUE_WIDTH: f32 = 72.0;
pub(super) const HEIGHT: f32 = ROW_HEIGHT * ROWS as f32;

enum Due {
    None,
    Overdue,
    Soon,
}

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let card = graphene::Rect::new(0.0, 0.0, width, HEIGHT);
    let rounded = gsk::RoundedRect::from_rect(card, 12.0);
    snapshot.push_rounded_clip(&rounded);
    snapshot.append_color(&with_alpha(&foreground, 0.03), &card);
    snapshot.pop();
    let edge = with_alpha(&foreground, 0.12);
    snapshot.append_border(&rounded, &[1.0; 4], &[edge; 4]);

    let today = Local::now().date_naive();
    let rows = [
        (
            gettext("Get back to Kim about the handover"),
            Due::None,
            false,
        ),
        (
            gettext("Renew the certificate for staging"),
            Due::Overdue,
            false,
        ),
        (gettext("Take the keyboard to the office"), Due::Soon, false),
        (gettext("Order a new headset"), Due::None, true),
    ];
    for (index, (title, due, done)) in rows.iter().enumerate() {
        let top = index as f32 * ROW_HEIGHT;
        let middle = top + ROW_HEIGHT / 2.0;
        if index > 0 {
            snapshot.append_color(&edge, &graphene::Rect::new(0.0, top, width, 1.0));
        }
        append_check_box(snapshot, INSET, middle, *done, &foreground);
        let room = width - TITLE_X - INSET - DUE_WIDTH;
        let (title, color) = if *done {
            let markup = format!("<s>{}</s>", glib::markup_escape_text(title));
            (
                markup_layout(widget, &markup, Some(room)),
                with_alpha(&foreground, 0.55),
            )
        } else {
            (layout(widget, title, Some(room)), foreground)
        };
        append_layout(snapshot, &title, TITLE_X, middle, 0.0, &color);

        let (date, color) = match due {
            Due::None => continue,
            Due::Overdue => (today - Days::new(2), error_color()),
            Due::Soon => (today + Days::new(3), with_alpha(&foreground, 0.7)),
        };
        let date = layout(widget, &format_short_date(date), None);
        append_layout(snapshot, &date, width - INSET, middle, 1.0, &color);
    }
}

/// Close to libadwaita's colour of errors, which the app cannot look up.
fn error_color() -> gdk::RGBA {
    if adw::StyleManager::default().is_dark() {
        parse("#ff938c")
    } else {
        parse("#c01c28")
    }
}
