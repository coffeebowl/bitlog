//! A contact as its page shows it: the picture or initials beside the name,
//! then a card with a number, an address and a role in a project.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{graphene, gsk};

use super::{append_layout, fill_rounded, layout};
use crate::colors::{parse, with_alpha};
use crate::drawing::{self, append_dot};
use crate::format::contact_label;

const AVATAR: f32 = 56.0;
/// Between the name and the card.
const GAP: f32 = 16.0;
const ROW_HEIGHT: f32 = 52.0;
const ROWS: usize = 3;
const INSET: f32 = 14.0;
pub(super) const HEIGHT: f32 = AVATAR + GAP + ROW_HEIGHT * ROWS as f32;

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();

    let radius = AVATAR / 2.0;
    append_dot(snapshot, radius, radius, radius, &parse("#986a44"));
    let initials = drawing::layout(widget, "KM", 1.3, true);
    append_layout(
        snapshot,
        &initials,
        (radius, radius),
        (0.5, 0.5),
        &parse("#ffffff"),
    );
    let name_x = AVATAR + INSET;
    let name = drawing::layout(widget, "Kim Meyer", 1.2, true);
    append_layout(
        snapshot,
        &name,
        (name_x, radius - 2.0),
        (0.0, 1.0),
        &foreground,
    );
    let company = layout(widget, &gettext("Example Ltd"), None);
    let dim = with_alpha(&foreground, 0.55);
    append_layout(snapshot, &company, (name_x, radius + 2.0), (0.0, 0.0), &dim);

    let top = AVATAR + GAP;
    let card = graphene::Rect::new(0.0, top, width, ROW_HEIGHT * ROWS as f32);
    fill_rounded(snapshot, card, 12.0, &with_alpha(&foreground, 0.03));
    let edge = with_alpha(&foreground, 0.12);
    snapshot.append_border(
        &gsk::RoundedRect::from_rect(card, 12.0),
        &[1.0; 4],
        &[edge; 4],
    );
    let rows = [
        (contact_label("mobile"), "+49 170 1234567".to_owned(), false),
        (
            contact_label("work"),
            "kim.meyer@example.com".to_owned(),
            false,
        ),
        (gettext("Webshop"), gettext("Product Owner"), true),
    ];
    for (index, (title, value, project)) in rows.iter().enumerate() {
        let row_top = top + index as f32 * ROW_HEIGHT;
        if index > 0 {
            snapshot.append_color(&edge, &graphene::Rect::new(0.0, row_top, width, 1.0));
        }
        let middle = row_top + ROW_HEIGHT / 2.0;
        let mut x = INSET;
        if *project {
            append_dot(snapshot, x + 4.0, middle, 4.5, &parse("#3584e4"));
            x += 20.0;
        }
        let room = Some(width - x - INSET);
        let title = drawing::layout(widget, title, 0.8, false);
        append_layout(snapshot, &title, (x, middle - 1.0), (0.0, 1.0), &dim);
        let value = layout(widget, value, room);
        append_layout(snapshot, &value, (x, middle + 1.0), (0.0, 0.0), &foreground);
    }
}
