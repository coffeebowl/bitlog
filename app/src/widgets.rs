//! Small helpers for widgets and actions that several pages share.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, glib};

/// The parameter of an action about `what`, as in "notes", parsed from the
/// string it is passed as.
pub fn param<T: FromStr>(param: Option<&glib::Variant>, what: &str) -> T {
    param
        .and_then(|param| param.str()?.parse().ok())
        .unwrap_or_else(|| panic!("actions about {what} take them as strings"))
}

/// Adds the style class `class` to `widget`, or removes it.
pub fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// How long what moves in the app glides or fades, in milliseconds.
const ANIMATION_MS: u32 = 250;

/// An animation from 0 to 1 that redraws `widget` at every step, and lays
/// it out again if `relayout`.
pub fn redraw_animation(widget: &impl IsA<gtk::Widget>, relayout: bool) -> adw::TimedAnimation {
    let widget = widget.as_ref();
    let target = adw::CallbackAnimationTarget::new(glib::clone!(
        #[weak]
        widget,
        move |_| {
            if relayout {
                widget.queue_allocate();
            }
            widget.queue_draw();
        }
    ));
    adw::TimedAnimation::builder()
        .widget(widget)
        .value_from(0.0)
        .value_to(1.0)
        .duration(ANIMATION_MS)
        .easing(adw::Easing::EaseOutCubic)
        .target(&target)
        .build()
}

/// Sets `field` to `entered` if that differs from `shown`, the value the
/// dialog started with, so that a value changed elsewhere meanwhile is kept
/// unless the user changed it too.
pub fn changed<T: PartialEq>(field: &mut T, shown: &T, entered: T) {
    if entered != *shown {
        *field = entered;
    }
}

/// What [`Choices`] needs of a dropdown: `gtk::DropDown` and `adw::ComboRow`
/// work alike, but have no type in common.
pub trait Dropdown {
    fn set_names(&self, names: &[&str]);
    fn set_selected(&self, index: u32);
    fn selected(&self) -> u32;
}

impl Dropdown for gtk::DropDown {
    fn set_names(&self, names: &[&str]) {
        self.set_model(Some(&gtk::StringList::new(names)));
    }

    fn set_selected(&self, index: u32) {
        gtk::DropDown::set_selected(self, index);
    }

    fn selected(&self) -> u32 {
        gtk::DropDown::selected(self)
    }
}

impl Dropdown for adw::ComboRow {
    fn set_names(&self, names: &[&str]) {
        self.set_model(Some(&gtk::StringList::new(names)));
    }

    fn set_selected(&self, index: u32) {
        ComboRowExt::set_selected(self, index);
    }

    fn selected(&self) -> u32 {
        ComboRowExt::selected(self)
    }
}

/// What a dropdown offers, in its order.
#[derive(Debug)]
pub struct Choices<T> {
    items: RefCell<Vec<T>>,
    /// Set while the dropdown is filled, so that it counts as no choice.
    filling: Cell<bool>,
}

impl<T> Default for Choices<T> {
    fn default() -> Self {
        Self {
            items: RefCell::default(),
            filling: Cell::default(),
        }
    }
}

impl<T: Clone + PartialEq> Choices<T> {
    /// Offers `items` in `dropdown`, shown by `name`, with `selected` chosen
    /// or else the first. The list is only replaced if it changed.
    pub fn fill(
        &self,
        dropdown: &impl Dropdown,
        items: Vec<T>,
        selected: &T,
        name: impl Fn(&T) -> String,
    ) {
        let index = items.iter().position(|item| item == selected).unwrap_or(0);
        self.filling.set(true);
        if *self.items.borrow() != items {
            let names: Vec<String> = items.iter().map(name).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            dropdown.set_names(&names);
            self.items.replace(items);
        }
        dropdown.set_selected(u32::try_from(index).expect("dropdowns offer few items"));
        self.filling.set(false);
    }

    /// The item chosen in `dropdown`, `None` while it is being filled.
    pub fn chosen(&self, dropdown: &impl Dropdown) -> Option<T> {
        if self.filling.get() {
            return None;
        }
        let index = usize::try_from(dropdown.selected()).ok()?;
        self.items.borrow().get(index).cloned()
    }
}

/// Saves what is being typed a second after the last change.
#[derive(Debug, Default)]
pub struct SaveTimer(Rc<RefCell<Option<glib::SourceId>>>);

impl SaveTimer {
    /// Runs `save` in a second, instead of what was to run.
    pub fn schedule(&self, save: impl FnOnce() + 'static) {
        self.cancel();
        let pending = self.0.clone();
        let source = glib::timeout_add_local_once(Duration::from_secs(1), move || {
            // Removing a source that ran would panic.
            pending.take();
            save();
        });
        self.0.replace(Some(source));
    }

    /// Stops what was to run. Returns whether there was something to save.
    pub fn cancel(&self) -> bool {
        self.0.take().map(glib::SourceId::remove).is_some()
    }
}

/// A handle that shows a row can be dragged, for `make_movable`.
pub fn drag_handle() -> gtk::Image {
    let handle = gtk::Image::builder()
        .icon_name("list-drag-handle-symbolic")
        .tooltip_text(gettext("Drag to Move"))
        .css_classes(["dim-label"])
        .build();
    handle.set_cursor_from_name(Some("grab"));
    handle
}

/// Lets `row` be dragged as `value` onto other rows set up this way, and
/// marks where it would go. `accepts` tells whether a value may go next to
/// `row`; `on_drop` moves it there, before `row` or after it if told so.
///
/// With a mouse the whole row can be dragged. On touch screens dragging
/// starts at `handle` or with a long press, so that swiping still scrolls
/// and tapping still works.
pub fn make_movable(
    row: &impl IsA<gtk::Widget>,
    handle: &impl IsA<gtk::Widget>,
    value: String,
    accepts: impl Fn(&str) -> bool + 'static,
    on_drop: impl Fn(String, bool) + 'static,
) {
    let row = row.upcast_ref::<gtk::Widget>();
    add_drag_source(row, handle.upcast_ref(), value.clone());
    add_drop_target(row, value, accepts, on_drop);
}

/// Lets `row` be dragged as `value`, see `make_movable`.
fn add_drag_source(row: &gtk::Widget, handle: &gtk::Widget, value: String) {
    // Where the row was grabbed, and whether a finger holds it so that it
    // may be dragged.
    let grab = Rc::new(Cell::new((0.0, 0.0)));
    let held = Rc::new(Cell::new(false));

    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    // Held, the row neither scrolls nor takes the tap when let go.
    let hold = |gesture: &gtk::Gesture, held: &Cell<bool>| {
        held.set(true);
        gesture.set_state(gtk::EventSequenceState::Claimed);
    };
    let touch = gtk::GestureClick::builder().touch_only(true).build();
    touch.group_with(&source);
    touch.connect_pressed(glib::clone!(
        #[weak]
        row,
        #[weak]
        handle,
        #[strong]
        held,
        move |gesture, _, x, _| {
            // The handle is small, so a finger may touch next to it.
            let on_handle = handle.compute_bounds(&row).is_some_and(|bounds| {
                (f64::from(bounds.x()) - 12.0..=f64::from(bounds.x() + bounds.width()) + 12.0)
                    .contains(&x)
            });
            held.set(false);
            if on_handle {
                hold(gesture.upcast_ref(), &held);
            }
        }
    ));
    let long_press = gtk::GestureLongPress::builder().touch_only(true).build();
    long_press.group_with(&source);
    long_press.connect_pressed(glib::clone!(
        #[strong]
        held,
        move |gesture, _, _| hold(gesture.upcast_ref(), &held)
    ));
    source.connect_prepare(glib::clone!(
        #[strong]
        held,
        #[strong]
        grab,
        #[strong]
        value,
        move |source, x, y| {
            let touch = source
                .current_event_device()
                .is_some_and(|device| device.source() == gdk::InputSource::Touchscreen);
            if touch && !held.get() {
                return None;
            }
            grab.set((x, y));
            Some(gdk::ContentProvider::for_value(&value.to_value()))
        }
    ));
    source.connect_drag_begin(glib::clone!(
        #[weak]
        row,
        #[strong]
        grab,
        move |_, drag| {
            // A copy of the row as it looks now, before it is dimmed.
            let picture =
                gtk::Picture::for_paintable(&gtk::WidgetPaintable::new(Some(&row)).current_image());
            picture.set_size_request(row.width(), row.height());
            picture.add_css_class("dragged-row");
            gtk::DragIcon::for_drag(drag).set_child(Some(&picture));
            let (x, y) = grab.get();
            drag.set_hotspot(x as i32, y as i32);
            row.add_css_class("dragging");
        }
    ));
    source.connect_drag_end(glib::clone!(
        #[weak]
        row,
        move |_, _, _| {
            held.set(false);
            row.remove_css_class("dragging");
        }
    ));
    row.add_controller(touch);
    row.add_controller(long_press);
    row.add_controller(source);
}

/// Lets values be dropped next to `row`, which holds `value`, see
/// `make_movable`.
fn add_drop_target(
    row: &gtk::Widget,
    value: String,
    accepts: impl Fn(&str) -> bool + 'static,
    on_drop: impl Fn(String, bool) + 'static,
) {
    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    target.set_preload(true);
    // The value dragged, and whether it goes after this row, if it may go
    // here at all.
    let place = Rc::new(glib::clone!(
        #[weak]
        row,
        #[upgrade_or]
        None,
        move |target: &gtk::DropTarget, y: f64| -> Option<(String, bool)> {
            let dragged: String = target.value()?.get().ok()?;
            (dragged != value && accepts(&dragged))
                .then(|| (dragged, y > f64::from(row.height()) / 2.0))
        }
    ));
    let unmark = |row: &gtk::Widget| {
        row.remove_css_class("drop-before");
        row.remove_css_class("drop-after");
    };
    target.connect_motion(glib::clone!(
        #[weak]
        row,
        #[strong]
        place,
        #[upgrade_or]
        gdk::DragAction::empty(),
        move |target, _, y| {
            let after = place(target, y).map(|(_, after)| after);
            set_class(&row, "drop-before", after == Some(false));
            set_class(&row, "drop-after", after == Some(true));
            if after.is_some() {
                gdk::DragAction::MOVE
            } else {
                gdk::DragAction::empty()
            }
        }
    ));
    target.connect_leave(glib::clone!(
        #[weak]
        row,
        move |_| unmark(&row)
    ));
    let on_drop = Rc::new(on_drop);
    target.connect_drop(glib::clone!(
        #[weak]
        row,
        #[upgrade_or]
        false,
        move |target, _, _, y| {
            unmark(&row);
            let Some((dragged, after)) = place(target, y) else {
                return false;
            };
            // Moving rebuilds the list with this row, so it waits until the
            // drop is done.
            let on_drop = on_drop.clone();
            glib::idle_add_local_once(move || on_drop(dragged, after));
            true
        }
    ));
    row.add_controller(target);
}

#[cfg(test)]
mod tests {
    use bitlog_core::NotePath;

    use super::*;

    #[test]
    fn parses_action_parameters() {
        let note: NotePath = param(
            Some(&"projects/webshop/notes/ideas.md".to_variant()),
            "notes",
        );
        assert_eq!(note.to_string(), "projects/webshop/notes/ideas.md");
        let day: chrono::NaiveDate = param(Some(&"2026-09-21".to_variant()), "days");
        assert_eq!(day.to_string(), "2026-09-21");
    }

    #[test]
    #[should_panic(expected = "actions about days")]
    fn rejects_wrong_parameters() {
        let _: chrono::NaiveDate = param(Some(&"yesterday".to_variant()), "days");
    }
}
