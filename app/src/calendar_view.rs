use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Days, Local, Months, NaiveDate};
use gettextrs::gettext;
use gtk::{glib, pango};
use knotbook_core::Vault;

use crate::format::{capitalize, format_date, format_duration, format_full_date};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/calendar_view.ui")]
    pub struct CalendarView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The first day of the month shown.
        pub month: Cell<NaiveDate>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub grid: TemplateChild<gtk::Grid>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CalendarView {
        const NAME: &'static str = "KnotbookCalendarView";
        type Type = super::CalendarView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CalendarView {}
    impl WidgetImpl for CalendarView {}
    impl NavigationPageImpl for CalendarView {}
}

glib::wrapper! {
    /// A month of the vault, one card per day.
    pub struct CalendarView(ObjectSubclass<imp::CalendarView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CalendarView {
    /// Shows the current month of `vault`.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
        self.show_month(Local::now().date_naive());
    }

    pub fn month(&self) -> NaiveDate {
        self.imp().month.get()
    }

    /// Shows the month `date` lies in.
    pub fn show_month(&self, date: NaiveDate) {
        let imp = self.imp();
        let first = date.with_day(1).expect("every month has a first day");
        imp.month.set(first);
        imp.window_title.set_title(&format_date(first, "%B"));
        imp.window_title.set_subtitle(&format_date(first, "%Y"));
        self.set_title(&format_date(first, "%B %Y"));

        let grid = &imp.grid;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        let vault = imp.vault.borrow();
        let vault = vault
            .as_ref()
            .expect("a month is only shown once a vault is open");
        let week_start = first
            - Days::new(
                first
                    .weekday()
                    .days_since(vault.config().week.first_day)
                    .into(),
            );
        for column in 0..7 {
            let weekday = format_date(week_start + Days::new(column), "%a");
            let label = gtk::Label::builder()
                .label(weekday)
                .css_classes(["caption-heading", "dim-label"])
                .build();
            grid.attach(&label, column as i32, 0, 1, 1);
        }

        let last = first + Months::new(1) - Days::new(1);
        let today = Local::now().date_naive();
        let mut date = week_start;
        let mut index = 0;
        // Whole weeks, from the one with the first to the one with the last day.
        while date <= last || index % 7 != 0 {
            let card = day_card(vault, date, date.month() == first.month(), date == today);
            grid.attach(&card, index % 7, index / 7 + 1, 1, 1);
            date = date + Days::new(1);
            index += 1;
        }
    }
}

/// A card for `date` with its hours, location and kind, which opens the day.
fn day_card(vault: &Vault, date: NaiveDate, in_month: bool, is_today: bool) -> gtk::Button {
    let label = |text: &str, classes: &[&str]| {
        gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(classes)
            .build()
    };
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_start(2)
        .build();
    let number_classes: &[&str] = if is_today {
        &["heading", "accent"]
    } else {
        &["heading"]
    };
    content.append(&label(&date.day().to_string(), number_classes));
    match vault.load_day(date) {
        Ok(Some((day, _))) => {
            content.append(&label(
                &format_duration(day.working_time(vault.projects())),
                &["caption"],
            ));
            if let Some(key) = &day.location {
                content.append(&label(
                    vault.config().location_name(key),
                    &["caption", "dim-label"],
                ));
            }
            content.append(&label(&capitalize(&day.kind), &["caption", "dim-label"]));
        }
        Ok(None) => {}
        Err(err) => {
            let error = label(&gettext("Cannot read"), &["caption", "error"]);
            error.set_tooltip_text(Some(&err.to_string()));
            content.append(&error);
        }
    }

    let card = gtk::Button::builder()
        .child(&content)
        .height_request(84)
        .action_name("win.show-day")
        .action_target(&date.to_string().to_variant())
        .css_classes(["card"])
        .build();
    if !in_month {
        card.add_css_class("dim-label");
    }
    card.update_property(&[gtk::accessible::Property::Label(&format_full_date(date))]);
    card
}
