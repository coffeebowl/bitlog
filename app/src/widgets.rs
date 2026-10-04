//! Small helpers for widgets and actions that several pages share.

use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

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

/// Sets `field` to `entered` if that differs from `shown`, the value the
/// dialog started with, so that a value changed elsewhere meanwhile is kept
/// unless the user changed it too.
pub fn changed<T: PartialEq>(field: &mut T, shown: &T, entered: T) {
    if entered != *shown {
        *field = entered;
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
