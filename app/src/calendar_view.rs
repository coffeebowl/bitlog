use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use std::collections::BTreeMap;

use chrono::{Datelike, Days, Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gdk, glib, pango};
use knotbook_core::{DayFile, Period, ProjectSlug, Vault, week_start};

use crate::colors::{UNKNOWN_PROJECT_COLOR, color_dot};
use crate::format::{format_date, format_duration, format_full_date, kind_name};
use crate::week_chart::{ChartDay, WeekChart};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/calendar_view.ui")]
    pub struct CalendarView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// A day in the month or week shown.
        pub date: Cell<NaiveDate>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub view_toggle: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub view_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub grid: TemplateChild<gtk::Grid>,
        #[template_child]
        pub week_total: TemplateChild<gtk::Label>,
        #[template_child]
        pub week_chart: TemplateChild<WeekChart>,
        #[template_child]
        pub legend: TemplateChild<gtk::FlowBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CalendarView {
        const NAME: &'static str = "KnotbookCalendarView";
        type Type = super::CalendarView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            WeekChart::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CalendarView {
        fn constructed(&self) {
            self.parent_constructed();
            self.view_toggle.connect_active_name_notify(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| {
                    if view.imp().vault.borrow().is_some() {
                        view.reload();
                    }
                }
            ));
        }
    }
    impl WidgetImpl for CalendarView {}
    impl NavigationPageImpl for CalendarView {}
}

glib::wrapper! {
    /// A month of the vault, one card per day, or a week as a chart.
    pub struct CalendarView(ObjectSubclass<imp::CalendarView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CalendarView {
    /// Shows the days of `vault` from the next call of `show` or `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    /// Shows the month or week `date` lies in.
    pub fn show(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.date.set(date);
        let page = imp
            .view_toggle
            .active_name()
            .expect("one view is always active");
        imp.view_stack.set_visible_child_name(&page);
        if page == "week" {
            self.show_week(date);
        } else {
            self.show_month(date);
        }
    }

    /// Reads the days shown again, they may have changed.
    pub fn reload(&self) {
        self.show(self.imp().date.get());
    }

    /// Shows the month or week `steps` months or weeks after the one shown.
    pub fn step(&self, steps: i32) {
        let period = if self.imp().view_toggle.active_name().as_deref() == Some("week") {
            Period::Week
        } else {
            Period::Month
        };
        let date = period.step(self.imp().date.get(), steps);
        self.show(date.expect("nobody steps this way to the end of the calendar"));
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the calendar is only shown once a vault is open")
    }

    fn show_month(&self, date: NaiveDate) {
        let imp = self.imp();
        let vault = self.vault();
        let vault = &*vault;
        let first_day = vault.config().week.first_day;
        let (first, last) = Period::Month.range(date, first_day);
        imp.window_title.set_title(&format_date(first, "%B"));
        imp.window_title.set_subtitle(&format_date(first, "%Y"));
        self.set_title(&format_date(first, "%B %Y"));

        let grid = &imp.grid;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        let week_start = week_start(first, first_day);
        for column in 0..7 {
            let weekday = format_date(week_start + Days::new(column), "%a");
            let label = gtk::Label::builder()
                .label(weekday)
                .css_classes(["caption-heading", "dim-label"])
                .build();
            grid.attach(&label, column as i32, 0, 1, 1);
        }

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

    fn show_week(&self, date: NaiveDate) {
        let imp = self.imp();
        let vault = self.vault();
        let (first, last) = Period::Week.range(date, vault.config().week.first_day);
        // Translators: A range of dates, as in "September 21 – 27".
        let title = gettext("{first} – {last}")
            .replace("{first}", &format_date(first, "%B %-d"))
            .replace("{last}", &format_date(last, "%B %-d"));
        imp.window_title.set_title(&title);
        imp.window_title.set_subtitle(&format_date(last, "%Y"));
        self.set_title(&title);

        let hex = |slug: &ProjectSlug| {
            vault
                .project(slug)
                .map_or(UNKNOWN_PROJECT_COLOR, |project| project.color.as_str())
                .to_owned()
        };
        let color = |slug: &ProjectSlug| {
            gdk::RGBA::parse(hex(slug)).expect("the core only accepts valid colours")
        };
        let today = Local::now().date_naive();
        let mut worked = TimeDelta::zero();
        let mut per_project = BTreeMap::new();
        let mut days = Vec::new();
        for offset in 0..7 {
            let date = first + Days::new(offset);
            // Unreadable days count as empty here, the month view shows why.
            let day = vault.load_day(date).ok().flatten().map(|file| file.day);
            let times = day
                .as_ref()
                .map(|day| day.time_per_project(vault.projects()))
                .unwrap_or_default();
            let working_time = day
                .as_ref()
                .map_or(TimeDelta::zero(), |day| day.working_time(vault.projects()));
            worked += working_time;
            for (slug, time) in &times {
                *per_project.entry(slug.clone()).or_insert(TimeDelta::zero()) += *time;
            }
            days.push(ChartDay {
                label: format_date(date, "%a %-d"),
                segments: times
                    .iter()
                    .map(|(slug, time)| (color(slug), hours(*time)))
                    .collect(),
                working_hours: (date <= today).then(|| hours(working_time)),
            });
        }
        let target = vault.config().week.target_hours as f32;
        imp.week_chart.set_week(days, target);

        let target_time = TimeDelta::minutes((target * 60.0).round() as i64);
        imp.week_total.set_label(&if target > 0.0 {
            // Translators: Hours worked in a week against its target, as in
            // "23 h 15 min of 32 h 0 min".
            gettext("{worked} of {target}")
                .replace("{worked}", &format_duration(worked))
                .replace("{target}", &format_duration(target_time))
        } else {
            format_duration(worked)
        });

        imp.legend.remove_all();
        let mut per_project: Vec<_> = per_project.into_iter().collect();
        per_project.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
        for (slug, time) in per_project {
            let name = vault.project_name(&slug);
            let markup = format!(
                "{} {} · {}",
                color_dot(&hex(&slug)),
                glib::markup_escape_text(name),
                glib::markup_escape_text(&format_duration(time)),
            );
            let label = gtk::Label::builder()
                .label(markup)
                .use_markup(true)
                .xalign(0.0)
                .build();
            imp.legend.append(&label);
        }
    }
}

fn hours(time: TimeDelta) -> f32 {
    time.num_minutes() as f32 / 60.0
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
        Ok(Some(DayFile { day, .. })) => {
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
            content.append(&label(&kind_name(&day.kind), &["caption", "dim-label"]));
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
