use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Days, Local, Months, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gdk, glib};
use knotbook_core::{ProjectSlug, Vault};

use crate::calendar_view::week_start;
use crate::format::{format_date, format_duration, format_share, format_short_date};
use crate::heatmap::Heatmap;
use crate::search_index::{ReportData, SearchIndex};
use crate::share_bar::ShareBar;
use crate::timeline::SEA_GREEN;

/// Colors for the categories, which have none of their own: the accent
/// colors of GNOME, used in turn.
const CATEGORY_COLORS: [&str; 9] = [
    "#3584e4", "#2190a4", "#3a944a", "#c88800", "#ed5b00", "#e62d42", "#d56199", "#9141ac",
    "#6f8396",
];

/// For projects that are not in the vault.
const UNKNOWN_COLOR: &str = "#9a9996";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Period {
    Week,
    Month,
    Year,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/reports_page.ui")]
    pub struct ReportsPage {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub index: RefCell<SearchIndex>,
        /// A day of the period shown, today until another one is chosen.
        pub date: Cell<Option<NaiveDate>>,
        /// Counts the lookups, so that one finishing after a newer one is
        /// dropped.
        pub lookups: Cell<u32>,
        pub rows: RefCell<Vec<(adw::PreferencesGroup, adw::ActionRow)>>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub period_toggle: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub projects_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub total_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub categories_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub category_bar: TemplateChild<ShareBar>,
        #[template_child]
        pub year_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub heatmap: TemplateChild<Heatmap>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReportsPage {
        const NAME: &'static str = "KnotbookReportsPage";
        type Type = super::ReportsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ShareBar::ensure_type();
            Heatmap::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReportsPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.period_toggle.connect_active_name_notify(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| {
                    if page.imp().vault.borrow().is_some() {
                        page.reload();
                    }
                }
            ));
        }
    }

    impl WidgetImpl for ReportsPage {}
    impl NavigationPageImpl for ReportsPage {}
}

glib::wrapper! {
    /// Where the time went in a week, month or year.
    pub struct ReportsPage(ObjectSubclass<imp::ReportsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ReportsPage {
    /// Shows the reports of `vault` from the next call of `show` or
    /// `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    pub fn set_index(&self, index: SearchIndex) {
        self.imp().index.replace(index);
    }

    /// Shows the week, month or year that holds `date`.
    pub fn show(&self, date: NaiveDate) {
        self.imp().date.set(Some(date));
        self.reload();
    }

    fn date(&self) -> NaiveDate {
        self.imp()
            .date
            .get()
            .unwrap_or_else(|| Local::now().date_naive())
    }

    /// Shows the period `steps` periods after the one shown.
    pub fn step(&self, steps: i32) {
        let date = self.date();
        let months = |count: i32| Months::new(count.unsigned_abs() * steps.unsigned_abs());
        let moved = match (self.period(), steps.is_negative()) {
            (Period::Week, _) => date.checked_add_signed(TimeDelta::weeks(steps.into())),
            (Period::Month, false) => date.checked_add_months(months(1)),
            (Period::Month, true) => date.checked_sub_months(months(1)),
            (Period::Year, false) => date.checked_add_months(months(12)),
            (Period::Year, true) => date.checked_sub_months(months(12)),
        };
        self.show(moved.expect("nobody steps this way to the end of the calendar"));
    }

    /// Looks up the period shown again and shows it.
    pub fn reload(&self) {
        let imp = self.imp();
        let vault = imp.vault.borrow().clone().expect("reports need a vault");
        let date = self.date();
        let period = self.range(&vault);
        self.show_title(period);
        let year = (
            NaiveDate::from_ymd_opt(date.year(), 1, 1).expect("years have a first day"),
            NaiveDate::from_ymd_opt(date.year(), 12, 31).expect("years have a last day"),
        );
        imp.year_group.set_title(&date.year().to_string());
        let lookup = imp.lookups.get() + 1;
        imp.lookups.set(lookup);
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let data = index.report(&vault, period, year).await;
                if page.imp().lookups.get() != lookup {
                    return;
                }
                match data {
                    Ok(data) => page.show_data(&vault, data, year),
                    Err(err) => glib::g_warning!("knotbook", "{err}"),
                }
            }
        ));
    }

    fn period(&self) -> Period {
        match self.imp().period_toggle.active_name().as_deref() {
            Some("week") => Period::Week,
            Some("year") => Period::Year,
            _ => Period::Month,
        }
    }

    /// The first and last day of the period shown.
    fn range(&self, vault: &Vault) -> (NaiveDate, NaiveDate) {
        let date = self.date();
        let first = match self.period() {
            Period::Week => week_start(date, vault.config().week.first_day),
            Period::Month => date.with_day(1).expect("every month has a first day"),
            Period::Year => date.with_ordinal(1).expect("every year has a first day"),
        };
        let next = match self.period() {
            Period::Week => first + Days::new(7),
            Period::Month => first + Months::new(1),
            Period::Year => first + Months::new(12),
        };
        (first, next.pred_opt().expect("the day before exists"))
    }

    fn show_title(&self, (first, last): (NaiveDate, NaiveDate)) {
        let title = &self.imp().window_title;
        let year = last.year().to_string();
        match self.period() {
            Period::Week => {
                title.set_title(&format!(
                    "{} – {}",
                    format_short_date(first),
                    format_short_date(last)
                ));
                title.set_subtitle(&year);
            }
            Period::Month => {
                // Translators: The name of a month, see the GLib
                // documentation of g_date_time_format() for the codes.
                title.set_title(&format_date(first, &gettext("%B")));
                title.set_subtitle(&year);
            }
            Period::Year => {
                title.set_title(&year);
                title.set_subtitle("");
            }
        }
    }

    fn show_data(&self, vault: &Vault, data: ReportData, year: (NaiveDate, NaiveDate)) {
        let imp = self.imp();
        for (group, row) in imp.rows.take() {
            group.remove(&row);
        }
        let is_break = |slug: &ProjectSlug| vault.project(slug).is_some_and(|p| p.is_break());
        let (breaks, work): (Vec<_>, Vec<_>) =
            data.times.iter().partition(|(slug, _)| is_break(slug));
        let total: TimeDelta = work.iter().map(|(_, time)| *time).sum();
        let breaks: TimeDelta = breaks.iter().map(|(_, time)| *time).sum();
        let mut rows = Vec::new();

        let most = work.first().map_or(TimeDelta::zero(), |(_, time)| *time);
        for (slug, time) in &work {
            let (name, color) = match vault.project(slug) {
                Some(project) => (project.name.clone(), project.color.as_str()),
                None => (slug.to_string(), UNKNOWN_COLOR),
            };
            let row = time_row(&dot(color), &name, *time, total);
            let color = gdk::RGBA::parse(color).expect("project colors are valid");
            row.add_suffix(&ShareBar::new(vec![(color, share(*time, most))]));
            rows.push((imp.projects_group.get(), row));
        }
        if work.is_empty() {
            let row = adw::ActionRow::builder()
                .title(gettext("Nothing logged in this period"))
                .build();
            row.add_css_class("dim-label");
            rows.push((imp.projects_group.get(), row));
        }
        if !breaks.is_zero() {
            let row = adw::ActionRow::builder().title(gettext("Breaks")).build();
            row.add_suffix(&duration_label(breaks));
            row.add_css_class("dim-label");
            rows.push((imp.projects_group.get(), row));
        }
        imp.total_label.set_label(&if total.is_zero() {
            String::new()
        } else {
            format_duration(total)
        });

        let mut categories = BTreeMap::<String, TimeDelta>::new();
        for (slug, time) in &work {
            // Blocks of projects that are not in the vault are work.
            let category = vault
                .project(slug)
                .map_or("work", |project| project.category.as_str());
            *categories.entry(category.to_owned()).or_default() += *time;
        }
        let mut categories: Vec<(String, TimeDelta)> = categories.into_iter().collect();
        categories.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
        let mut parts = Vec::new();
        for ((category, time), color) in categories.iter().zip(CATEGORY_COLORS.iter().cycle()) {
            let row = time_row(&dot(color), &capitalize(category), *time, total);
            rows.push((imp.categories_group.get(), row));
            let color = gdk::RGBA::parse(*color).expect("the category colors are valid");
            parts.push((color, share(*time, total)));
        }
        imp.category_bar.set_parts(parts);
        imp.categories_group.set_visible(!categories.is_empty());
        for (group, row) in &rows {
            group.add(row);
        }
        imp.rows.replace(rows);

        let mut days = BTreeMap::<NaiveDate, TimeDelta>::new();
        for (date, slug, time) in &data.year {
            if !is_break(slug) {
                *days.entry(*date).or_default() += *time;
            }
        }
        let days: Vec<(NaiveDate, TimeDelta)> = days.into_iter().collect();
        let green = gdk::RGBA::parse(SEA_GREEN).expect("the colour is valid");
        imp.heatmap
            .show(&days, green, year, vault.config().week.first_day);
    }
}

/// A row with a colored dot, a name, and the time and its share of `total`.
fn time_row(dot: &str, name: &str, time: TimeDelta, total: TimeDelta) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(format!("{dot}  {}", glib::markup_escape_text(name)))
        .build();
    row.add_suffix(&duration_label(time));
    let share = gtk::Label::builder()
        .label(format_share(time, total))
        .width_chars(5)
        .xalign(1.0)
        .css_classes(["dim-label", "numeric"])
        .build();
    row.add_suffix(&share);
    row
}

fn duration_label(time: TimeDelta) -> gtk::Label {
    gtk::Label::builder()
        .label(format_duration(time))
        .css_classes(["numeric"])
        .build()
}

/// A colored dot as Pango markup.
fn dot(color: &str) -> String {
    format!("<span foreground=\"{color}\">●</span>")
}

fn share(part: TimeDelta, whole: TimeDelta) -> f32 {
    if whole.is_zero() {
        0.0
    } else {
        part.num_minutes() as f32 / whole.num_minutes() as f32
    }
}

/// `text` with a capital first letter, as categories are usually written in
/// lowercase.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
