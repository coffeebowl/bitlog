//! A morning as the day view shows it: blocks on a time line, with a gap
//! and a break, the time that does not count.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{graphene, gsk};

use super::{append_layout, fill_rounded, layout};
use crate::colors::{parse, with_alpha};

/// Smaller than in the day view, to show a morning at a glance.
const MINUTE_HEIGHT: f32 = 0.9;
/// Room above the first and below the last hour line for its label.
const PADDING: f32 = 10.0;
const LABEL_WIDTH: f32 = 44.0;
const LINE_X: f32 = 56.0;
const BLOCK_X: f32 = 72.0;
const KNOT_RADIUS: f32 = 4.0;
/// Inside a block, before its label.
const LABEL_INSET: f32 = 12.0;
const FIRST_MINUTE: u32 = 9 * 60;
const LAST_MINUTE: u32 = 13 * 60;
pub(super) const HEIGHT: f32 = 2.0 * PADDING + (LAST_MINUTE - FIRST_MINUTE) as f32 * MINUTE_HEIGHT;

enum Kind {
    /// A block of a project with this colour.
    Project(&'static str),
    Break,
    Gap,
}

/// From and to which minute of the day, what, and its label.
fn spans() -> [(u32, u32, Kind, String); 5] {
    [
        (
            540,
            630,
            Kind::Project("#3584e4"),
            gettext("Webshop · Work"),
        ),
        (
            630,
            660,
            Kind::Project("#e5a50a"),
            gettext("Meetings · Overhead"),
        ),
        (660, 690, Kind::Gap, gettext("No block, not counted")),
        (
            690,
            735,
            Kind::Project("#33d17a"),
            gettext("Infrastructure · Work"),
        ),
        (735, 765, Kind::Break, gettext("Break, not counted")),
    ]
}

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();

    for minute in (FIRST_MINUTE..=LAST_MINUTE).step_by(60) {
        let y = y_of(minute);
        let line = graphene::Rect::new(LINE_X, y, width - LINE_X, 1.0);
        snapshot.append_color(&with_alpha(&foreground, 0.2), &line);
        let label = layout(widget, &format!("{:02}:00", minute / 60), None);
        append_layout(
            snapshot,
            &label,
            LABEL_WIDTH,
            y,
            1.0,
            &with_alpha(&foreground, 0.55),
        );
    }
    let line = graphene::Rect::new(LINE_X - 1.0, 0.0, 2.0, HEIGHT);
    snapshot.append_color(&with_alpha(&foreground, 0.3), &line);

    for (start, end, kind, label) in spans() {
        let area = graphene::Rect::new(
            BLOCK_X,
            y_of(start) + 1.0,
            width - BLOCK_X,
            (end - start) as f32 * MINUTE_HEIGHT - 2.0,
        );
        let color = match kind {
            Kind::Project(hex) => Some(parse(hex)),
            Kind::Break => Some(with_alpha(&foreground, 0.5)),
            Kind::Gap => None,
        };
        let text_color = if let Some(color) = &color {
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(area, 6.0));
            snapshot.append_color(&with_alpha(color, 0.18), &area);
            let stripe = graphene::Rect::new(area.x(), area.y(), 4.0, area.height());
            snapshot.append_color(color, &stripe);
            snapshot.pop();
            let knot = graphene::Rect::new(
                LINE_X - KNOT_RADIUS,
                y_of(start) - KNOT_RADIUS,
                2.0 * KNOT_RADIUS,
                2.0 * KNOT_RADIUS,
            );
            fill_rounded(snapshot, knot, KNOT_RADIUS, &foreground);
            foreground
        } else {
            with_alpha(&foreground, 0.55)
        };
        let label = layout(widget, &label, Some(area.width() - 2.0 * LABEL_INSET));
        let y = area.y() + area.height() / 2.0;
        append_layout(
            snapshot,
            &label,
            area.x() + LABEL_INSET,
            y,
            0.0,
            &text_color,
        );
    }
}

fn y_of(minute: u32) -> f32 {
    PADDING + (minute - FIRST_MINUTE) as f32 * MINUTE_HEIGHT
}
