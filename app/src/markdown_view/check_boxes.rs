//! Check boxes for tasks like `- [ ] Call Anna`. A click checks or
//! unchecks one by changing the text.

use std::ops::Range;

use gtk::graphene;
use gtk::prelude::*;

use super::decorations::char_location;
use super::styling::Styling;
use super::tags;
use crate::drawing::append_check_box;

/// The size of check boxes, as a share of the height of the font.
const SCALE: f32 = 0.8;

#[derive(Debug, Clone)]
pub(super) struct CheckBox {
    /// Where it is drawn: over the first character of what it stands for.
    at: i32,
    checked: bool,
    /// What a click replaces, and with what.
    toggle: (Range<i32>, String),
}

impl CheckBox {
    /// Shows `range` of the text as a check box, `checked` or not: its
    /// first two characters, which are as wide as the box or the `room` it
    /// takes, if that is more, with the second spaced out, and the others
    /// hidden. Not the first, as Pango spaces out characters at the start
    /// of a line by half. A click replaces `toggle` with `replacement`.
    pub(super) fn conceal(
        styling: &Styling,
        range: &Range<usize>,
        (checked, room): (bool, i32),
        (toggle, replacement): (Range<usize>, &str),
    ) -> Self {
        let buffer = &styling.buffer;
        let two = styling.text[range.clone()]
            .char_indices()
            .nth(2)
            .map_or(range.len(), |(index, _)| index);
        let width = styling.width(&styling.text[range.start..range.start + two]);
        let chars = styling.chars(range);
        let at = chars.start;
        tags::apply(buffer, tags::CONCEALED, at..at + 2);
        let spacing = tags::spacing(buffer, (size(styling.view) as i32).max(room) - width);
        tags::apply(buffer, &spacing, at + 1..at + 2);
        tags::apply(buffer, tags::HIDDEN, at + 2..chars.end);
        Self {
            at,
            checked,
            toggle: (styling.chars(&toggle), replacement.to_owned()),
        }
    }

    /// Where it is drawn in `view`, in buffer coordinates.
    fn bounds(&self, view: &gtk::TextView) -> graphene::Rect {
        let location = char_location(view, self.at);
        let size = size(view);
        let y = location.y() as f32 + (location.height() as f32 - size) / 2.0;
        graphene::Rect::new(location.x() as f32, y.round(), size, size)
    }

    /// Whether it is at `x`, `y` in `view`, in buffer coordinates, with a
    /// little room around it.
    pub(super) fn contains(&self, view: &gtk::TextView, x: i32, y: i32) -> bool {
        self.bounds(view)
            .inset_r(-3.0, -3.0)
            .contains_point(&graphene::Point::new(x as f32, y as f32))
    }

    /// What a click replaces, and with what.
    pub(super) fn toggle(&self) -> (Range<i32>, String) {
        self.toggle.clone()
    }

    pub(super) fn snapshot(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot) {
        append_check_box(snapshot, self.bounds(view), self.checked, &view.color());
    }
}

/// The size of a check box in the font of `view`.
fn size(view: &gtk::TextView) -> f32 {
    let height = view.create_pango_layout(Some("X")).pixel_size().1;
    (height as f32 * SCALE).round()
}
