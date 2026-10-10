//! Drawing helpers for the widgets that draw themselves.

use adw::prelude::*;
use gtk::{gdk, graphene, gsk, pango};

use crate::colors::with_alpha;

/// `text` in the font of `widget`, at `scale` of its size, and bold if
/// `bold`.
pub fn layout(widget: &impl IsA<gtk::Widget>, text: &str, scale: f64, bold: bool) -> pango::Layout {
    let layout = widget.create_pango_layout(Some(text));
    let attributes = pango::AttrList::new();
    if scale != 1.0 {
        attributes.insert(pango::AttrFloat::new_scale(scale));
    }
    if bold {
        attributes.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
    }
    layout.set_attributes(Some(&attributes));
    layout
}

/// Draws `layout` with the point `align` of it at `x`, `y`, as shares of its
/// width and height: `(0.0, 0.0)` is its top left corner, `(0.5, 0.5)` its
/// middle. Its top lies on a whole pixel, where text looks sharpest; across,
/// it may lie between pixels, as text is placed by fractions there.
pub fn append_layout(
    snapshot: &gtk::Snapshot,
    layout: &pango::Layout,
    (x, y): (f32, f32),
    (align_x, align_y): (f32, f32),
    color: &gdk::RGBA,
) {
    let (width, height) = layout.pixel_size();
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        x - width as f32 * align_x,
        (y - height as f32 * align_y).floor(),
    ));
    snapshot.append_layout(layout, color);
    snapshot.restore();
}

/// Fills `rect` in `color`, with corners rounded by `radius`.
pub fn fill_rounded(
    snapshot: &gtk::Snapshot,
    rect: graphene::Rect,
    radius: f32,
    color: &gdk::RGBA,
) {
    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, radius));
    snapshot.append_color(color, &rect);
    snapshot.pop();
}

/// A dot around `x`, `y`.
pub fn append_dot(snapshot: &gtk::Snapshot, x: f32, y: f32, radius: f32, color: &gdk::RGBA) {
    let bounds = graphene::Rect::new(x - radius, y - radius, 2.0 * radius, 2.0 * radius);
    fill_rounded(snapshot, bounds, radius, color);
}

/// A check box in the square `bounds` as GTK draws one: an outline in a
/// lighter `foreground`, or filled in the accent colour with a tick.
pub fn append_check_box(
    snapshot: &gtk::Snapshot,
    bounds: graphene::Rect,
    checked: bool,
    foreground: &gdk::RGBA,
) {
    let (x, y, size) = (bounds.x(), bounds.y(), bounds.width());
    if checked {
        let accent = adw::StyleManager::default().accent_color_rgba();
        fill_rounded(snapshot, bounds, size / 4.0, &accent);
        let tick = gsk::PathBuilder::new();
        tick.move_to(x + size * 0.25, y + size * 0.52);
        tick.line_to(x + size * 0.43, y + size * 0.7);
        tick.line_to(x + size * 0.75, y + size * 0.32);
        let stroke = gsk::Stroke::new(size / 8.0);
        stroke.set_line_cap(gsk::LineCap::Round);
        stroke.set_line_join(gsk::LineJoin::Round);
        snapshot.append_stroke(&tick.to_path(), &stroke, &gdk::RGBA::WHITE);
    } else {
        let outline = gsk::RoundedRect::from_rect(bounds, size / 4.0);
        let color = with_alpha(foreground, 0.4);
        snapshot.append_border(&outline, &[1.5; 4], &[color; 4]);
    }
}
