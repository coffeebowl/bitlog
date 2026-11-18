//! Views in a scrolled window. One that does not scroll itself, like the
//! note of a day, is as high as its text, of which the window shows only
//! some: it draws only what is in sight, again as the window scrolls.

use gtk::prelude::*;
use gtk::{gdk, glib};

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
