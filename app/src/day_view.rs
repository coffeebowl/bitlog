use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeDelta};
use gettextrs::gettext;
use gtk::glib;
use knotbook_core::{Day, Vault};

use crate::timeline::Timeline;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/day_view.ui")]
    pub struct DayView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub date: Cell<NaiveDate>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub kind_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub location_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub working_time_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub work_hours_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub timeline: TemplateChild<Timeline>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayView {
        const NAME: &'static str = "KnotbookDayView";
        type Type = super::DayView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Timeline::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DayView {}
    impl WidgetImpl for DayView {}
    impl NavigationPageImpl for DayView {}
}

glib::wrapper! {
    /// One day of the vault: its details and its blocks.
    pub struct DayView(ObjectSubclass<imp::DayView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DayView {
    /// Shows today of `vault`.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
        self.show_date(Local::now().date_naive());
    }

    pub fn date(&self) -> NaiveDate {
        self.imp().date.get()
    }

    pub fn show_date(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.date.set(date);
        let (weekday, full_date) = date_titles(date);
        imp.window_title.set_title(&weekday);
        imp.window_title.set_subtitle(&full_date);
        self.set_title(&weekday);

        let vault = imp.vault.borrow();
        let vault = vault
            .as_ref()
            .expect("a day is only shown once a vault is open");
        match vault.load_day(date) {
            Ok(Some((day, _))) => {
                self.show_details(vault, &day);
                imp.stack.set_visible_child_name("day");
            }
            Ok(None) => imp.stack.set_visible_child_name("empty"),
            Err(err) => {
                imp.error_page
                    .set_description(Some(&glib::markup_escape_text(&err.to_string())));
                imp.stack.set_visible_child_name("error");
            }
        }
    }

    fn show_details(&self, vault: &Vault, day: &Day) {
        let imp = self.imp();
        imp.kind_label.set_label(&capitalize(&day.kind));
        let location = day.location.as_ref().map(|key| {
            vault
                .config()
                .locations
                .get(key)
                .cloned()
                // Hand-edited files may use a key the configuration lacks.
                .unwrap_or_else(|| key.to_string())
        });
        imp.location_label
            .set_label(location.as_deref().unwrap_or("–"));
        imp.working_time_label
            .set_label(&format_duration(day.working_time(vault.projects())));
        let hours = match (day.work_start, day.work_end) {
            (Some(start), Some(end)) => format!("{}–{}", format_time(start), format_time(end)),
            _ => String::new(),
        };
        imp.work_hours_label.set_visible(!hours.is_empty());
        imp.work_hours_label.set_label(&hours);
        let is_today = day.date == Local::now().date_naive();
        imp.timeline.set_day(vault, day, is_today);
    }
}

/// The weekday and the full date of `date`, in the user's language.
fn date_titles(date: NaiveDate) -> (String, String) {
    let date = glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    )
    .expect("every date chrono knows is valid in GLib");
    let format = |format: &str| {
        date.format(format)
            .expect("the date format is valid")
            .to_string()
    };
    // Translators: A date without the weekday, as in "September 22, 2026".
    // See the GLib documentation of g_date_time_format() for the codes.
    (format("%A"), format(&gettext("%B %-d, %Y")))
}

fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

fn format_duration(duration: TimeDelta) -> String {
    let minutes = duration.num_minutes();
    // Translators: A duration, as in "7 h 45 min".
    gettext("{hours} h {minutes} min")
        .replace("{hours}", &(minutes / 60).to_string())
        .replace("{minutes}", &(minutes % 60).to_string())
}

/// `kind` is free text in the file, usually lowercase like `work`.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
