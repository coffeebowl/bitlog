//! The four project categories as a matrix of whether the time counts as
//! working time and whether it is work, with an example in each field.

use bitlog_core::ProjectCategory;
use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{glib, graphene, gsk};

use super::{append_layout, layout, markup_layout};
use crate::colors::{parse, with_alpha};
use crate::format::project_category_name;

const HEADER_HEIGHT: f32 = 28.0;
const FIELD_HEIGHT: f32 = 56.0;
const GAP: f32 = 6.0;
/// Of the text from the left edge of a field.
const INSET: f32 = 14.0;
pub(super) const HEIGHT: f32 = HEADER_HEIGHT + 2.0 * FIELD_HEIGHT + GAP;

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let dim = with_alpha(&foreground, 0.55);

    let rows = [gettext("Is work"), gettext("Isn’t work")].map(|text| layout(widget, &text, None));
    let label_width = rows.iter().map(|row| row.pixel_size().0).max().unwrap_or(0) as f32 + 12.0;
    let field_width = (width - label_width - GAP) / 2.0;
    let left = |column: usize| label_width + column as f32 * (field_width + GAP);
    let top = |row: usize| HEADER_HEIGHT + row as f32 * (FIELD_HEIGHT + GAP);

    for (column, text) in [gettext("Counts"), gettext("Doesn’t count")]
        .iter()
        .enumerate()
    {
        let header = layout(widget, text, Some(field_width));
        let position = (left(column), HEADER_HEIGHT / 2.0);
        append_layout(snapshot, &header, position, (0.0, 0.5), &dim);
    }
    for (row, label) in rows.iter().enumerate() {
        let position = (0.0, top(row) + FIELD_HEIGHT / 2.0);
        append_layout(snapshot, label, position, (0.0, 0.5), &dim);
    }

    // In the order of the matrix, by rows, as the default projects.
    let fields = [
        (parse("#3584e4"), gettext("Customers")),
        (parse("#33d17a"), gettext("Side projects")),
        (parse("#2190a4"), gettext("Business trips")),
        (with_alpha(&foreground, 0.5), gettext("Lunch")),
    ];
    for (index, (category, (color, example))) in
        ProjectCategory::ALL.iter().zip(&fields).enumerate()
    {
        let (x, y) = (left(index % 2), top(index / 2));
        let area = graphene::Rect::new(x, y, field_width, FIELD_HEIGHT);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(area, 6.0));
        snapshot.append_color(&with_alpha(color, 0.18), &area);
        snapshot.append_color(color, &graphene::Rect::new(x, y, 4.0, FIELD_HEIGHT));
        snapshot.pop();

        let room = Some(field_width - 2.0 * INSET);
        let middle = y + FIELD_HEIGHT / 2.0;
        let name = glib::markup_escape_text(&project_category_name(*category));
        let name = markup_layout(widget, &format!("<b>{name}</b>"), room);
        append_layout(
            snapshot,
            &name,
            (x + INSET, middle),
            (0.0, 1.0),
            &foreground,
        );
        let example = layout(widget, example, room);
        append_layout(snapshot, &example, (x + INSET, middle), (0.0, 0.0), &dim);
    }
}
