use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Period, ProjectSlug, Vault};
use bitlog_index::export;
use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::alert::show_error;
use crate::colors::{UNKNOWN_PROJECT_COLOR, color_dot, lightness, mix, sea_green};
use crate::format::{capitalize, format_date, format_duration, format_share, format_short_date};
use crate::heatmap::Heatmap;
use crate::search_index::{ReportData, SearchIndex};
use crate::share_bar::ShareBar;

/// Colors for the categories, which have none of their own: the accent
/// colors of GNOME, used in turn.
const CATEGORY_COLORS: [&str; 9] = [
    "#3584e4", "#2190a4", "#3a944a", "#c88800", "#ed5b00", "#e62d42", "#d56199", "#9141ac",
    "#6f8396",
];

/// The size of a day in the heatmap of a year, and the most it grows to in
/// the single row of a week or month.
const HEATMAP_CELL: f32 = 8.0;
const HEATMAP_ROW_CELL: f32 = 32.0;

/// What the export menu offers.
#[derive(Debug, Clone, Copy)]
enum Export {
    Blocks,
    Week,
    Remote,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/reports_page.ui")]
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
        pub heatmap: TemplateChild<Heatmap>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReportsPage {
        const NAME: &'static str = "BitLogReportsPage";
        type Type = super::ReportsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ShareBar::ensure_type();
            Heatmap::ensure_type();
            klass.bind_template();
            for (action, export) in [
                ("reports.export-blocks", Export::Blocks),
                ("reports.export-week", Export::Week),
                ("reports.export-remote", Export::Remote),
            ] {
                klass.install_action_async(action, None, move |page, _, _| async move {
                    page.export(export).await;
                });
            }
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
        let moved = self.period().step(self.date(), steps);
        self.show(moved.expect("nobody steps this way to the end of the calendar"));
    }

    /// Looks up the period shown again and shows it.
    pub fn reload(&self) {
        let imp = self.imp();
        let vault = imp.vault.borrow().clone().expect("reports need a vault");
        let period = self.range(&vault);
        self.show_title(period);
        // A report covers one week.
        self.action_set_enabled("reports.export-week", self.period() == Period::Week);
        // A year in a column per week, a week or month in a single row with
        // the days as big as the width allows.
        let year = self.period() == Period::Year;
        imp.heatmap.set_single_row(!year);
        imp.heatmap
            .set_cell_size(if year { HEATMAP_CELL } else { HEATMAP_ROW_CELL });
        let lookup = imp.lookups.get() + 1;
        imp.lookups.set(lookup);
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let data = index.report(&vault, period).await;
                if page.imp().lookups.get() != lookup {
                    return;
                }
                match data {
                    Ok(data) => page.show_data(&vault, data, period),
                    Err(err) => glib::g_warning!("bitlog", "{err}"),
                }
            }
        ));
    }

    /// Writes `what` of the period shown to the exports folder and says so
    /// in a toast that leads to the file.
    async fn export(&self, what: Export) {
        let imp = self.imp();
        let vault = imp.vault.borrow().clone().expect("reports need a vault");
        let index = imp.index.borrow().clone();
        let period = self.range(&vault);
        let export = match what {
            Export::Blocks => index
                .blocks_csv(&vault, period)
                .await
                .map(|text| (export::blocks_file_name(Some(period)), text))
                .map_err(|err| err.to_string()),
            Export::Week => export::week_report(&vault, period.0)
                .map(|text| (export::week_file_name(period.0), text))
                .map_err(|err| err.to_string()),
            Export::Remote => index
                .remote_days_csv(&vault)
                .await
                .map(|text| (export::REMOTE_DAYS_FILE.to_owned(), text))
                .map_err(|err| err.to_string()),
        };
        let written = export.and_then(|(name, text)| {
            vault
                .write_export(&name, &text)
                .map_err(|err| err.to_string())
        });
        match written {
            Ok(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let toast = adw::Toast::builder()
                    .title(gettext("Exported {name}").replace("{name}", &name))
                    .button_label(gettext("_Show File"))
                    .build();
                let file = gio::File::for_path(&path);
                toast.connect_button_clicked(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_| {
                        let window = page.root().and_downcast::<gtk::Window>();
                        gtk::FileLauncher::new(Some(&file)).open_containing_folder(
                            window.as_ref(),
                            None::<&gio::Cancellable>,
                            |result| {
                                if let Err(err) = result {
                                    glib::g_warning!("bitlog", "{err}");
                                }
                            },
                        );
                    }
                ));
                self.ancestor(adw::ToastOverlay::static_type())
                    .and_downcast::<adw::ToastOverlay>()
                    .expect("pages lie in the window's toast overlay")
                    .add_toast(toast);
            }
            Err(message) => show_error(self, &gettext("Cannot Export"), &message),
        }
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
        self.period()
            .range(self.date(), vault.config().week.first_day)
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

    fn show_data(&self, vault: &Vault, data: ReportData, period: (NaiveDate, NaiveDate)) {
        let imp = self.imp();
        for (group, row) in imp.rows.take() {
            group.remove(&row);
        }
        let (breaks, work): (Vec<_>, Vec<_>) = data
            .times
            .iter()
            .partition(|(slug, _)| vault.is_break(slug));
        let total: TimeDelta = work.iter().map(|(_, time)| *time).sum();
        let breaks: TimeDelta = breaks.iter().map(|(_, time)| *time).sum();
        let mut rows = Vec::new();

        let most = work.first().map_or(TimeDelta::zero(), |(_, time)| *time);
        for (slug, time) in &work {
            let (name, color) = match vault.project(slug) {
                Some(project) => (project.name.clone(), project.color.as_str()),
                None => (slug.to_string(), UNKNOWN_PROJECT_COLOR),
            };
            let row = time_row(&color_dot(color), &name, *time, total);
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
            let row = time_row(&color_dot(color), &capitalize(category), *time, total);
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

        // Each day in the colors of its projects, mixed by the time spent on
        // them, all as light as the projects are on average, so that only
        // the time makes a day stand out.
        let lightness = average_lightness(vault);
        let mut days = BTreeMap::<NaiveDate, Vec<(gdk::RGBA, TimeDelta)>>::new();
        for (date, slug, time) in &data.days {
            if !vault.is_break(slug) {
                days.entry(*date)
                    .or_default()
                    .push((project_color(vault, slug), *time));
            }
        }
        let days = days.into_iter().map(|(date, parts)| {
            let time = parts.iter().map(|(_, time)| *time).sum();
            let parts: Vec<_> = parts
                .into_iter()
                .map(|(color, time)| (color, time.num_minutes() as f32))
                .collect();
            (date, time, mix(&parts, lightness))
        });
        imp.heatmap
            .show(days, period, vault.config().week.first_day);
    }
}

/// The color of the project `slug`, gray for one the vault does not know.
fn project_color(vault: &Vault, slug: &ProjectSlug) -> gdk::RGBA {
    let color = vault
        .project(slug)
        .map_or(UNKNOWN_PROJECT_COLOR, |project| project.color.as_str());
    gdk::RGBA::parse(color).expect("project colors are valid")
}

/// How light the colors of the projects of `vault` are on average, breaks
/// left out.
fn average_lightness(vault: &Vault) -> f32 {
    let lightnesses: Vec<f32> = vault
        .projects()
        .iter()
        .filter(|project| !project.is_break())
        .map(|project| lightness(&project_color(vault, &project.slug)))
        .collect();
    if lightnesses.is_empty() {
        lightness(&sea_green())
    } else {
        lightnesses.iter().sum::<f32>() / lightnesses.len() as f32
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

fn share(part: TimeDelta, whole: TimeDelta) -> f32 {
    if whole.is_zero() {
        0.0
    } else {
        part.num_minutes() as f32 / whole.num_minutes() as f32
    }
}
