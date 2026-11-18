//! Views in a scrolled window. One that does not scroll itself, like the
//! note of a day, is as high as its text, of which the window shows only
//! some: it draws only what is in sight, again as the window scrolls, and
//! keeps the cursor in sight, as views that scroll themselves do.

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

/// The scrolled window around `view`, if there is one.
fn scrolled_window(view: &gtk::TextView) -> Option<gtk::ScrolledWindow> {
    view.ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast()
}

/// Draws `view` again as the window around it scrolls: GTK only moves a
/// view that does not scroll itself. Returns what to disconnect.
pub(super) fn redraw_on_scroll(
    view: &gtk::TextView,
) -> Option<(gtk::Adjustment, glib::SignalHandlerId)> {
    let adjustment = scrolled_window(view)?.vadjustment();
    let handler = adjustment.connect_value_changed(glib::clone!(
        #[weak]
        view,
        move |_| view.queue_draw()
    ));
    Some((adjustment, handler))
}

/// What of the text of `view` is in sight, in buffer coordinates.
pub(super) fn in_sight(view: &gtk::TextView) -> gdk::Rectangle {
    let visible = view.visible_rect();
    let Some(shown) = scrolled_window(view).and_then(|scrolled| scrolled.compute_bounds(view))
    else {
        return visible;
    };
    let (x, y) = view.window_to_buffer_coords(
        gtk::TextWindowType::Widget,
        shown.x() as i32,
        shown.y() as i32,
    );
    let shown = gdk::Rectangle::new(x, y, shown.width() as i32, shown.height() as i32);
    visible
        .intersect(&shown)
        .unwrap_or_else(|| gdk::Rectangle::new(visible.x(), visible.y(), 0, 0))
}

/// Scrolls the window around `view` as little as it takes for the cursor
/// to be in sight, unless the view scrolls itself.
pub(super) fn follow_cursor(view: &gtk::TextView) {
    let Some(scrolled) = scrolled_window(view) else {
        return;
    };
    let adjustment = scrolled.vadjustment();
    if view.vadjustment().as_ref() == Some(&adjustment) {
        return;
    }
    let buffer = view.buffer();
    let (cursor, _) = view.cursor_locations(Some(&buffer.iter_at_mark(&buffer.get_insert())));
    let (x, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, cursor.x(), cursor.y());
    let Some(top) = view.compute_point(&scrolled, &graphene::Point::new(x as f32, y as f32)) else {
        return;
    };
    let top = f64::from(top.y());
    let below = top + f64::from(cursor.height()) - f64::from(scrolled.height());
    if top < 0.0 {
        adjustment.set_value(adjustment.value() + top);
    } else if below > 0.0 {
        adjustment.set_value(adjustment.value() + below);
    }
}
