use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use std::collections::BTreeMap;

use bitlog_core::{DayFile, Period, Vault, week_start};
use chrono::{Datelike, Days, Local, NaiveDate, TimeDelta, Weekday};
use gettextrs::gettext;
use gtk::{glib, pango};

use crate::colors::{color_dot, project_color, project_hex};
use crate::day_summary::{DaySummary, Details};
use crate::format::{
    format_date, format_duration, format_full_date, format_month, format_month_year, format_range,
    format_short_date, format_short_duration, kind_name,
};
use crate::share_bar::ShareBar;
use crate::week_chart::{ChartDay, WeekChart};
use crate::week_progress::WeekProgress;

/// How much of a day the month shows, by the room there is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Density {
    #[default]
    Full,
    /// With the hours shortened.
    Short,
    /// With the hours shortened, below the number of the day.
    Stacked,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/calendar_view.ui")]
    pub struct CalendarView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// A day in the month or week shown.
        pub date: Cell<NaiveDate>,
        pub(super) density: Cell<Density>,
        #[template_child]
        pub breakpoint_bin: TemplateChild<adw::BreakpointBin>,
        #[template_child]
        pub short_breakpoint: TemplateChild<adw::Breakpoint>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub today_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub view_toggle: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub view_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub month: TemplateChild<gtk::Box>,
        #[template_child]
        pub week_total: TemplateChild<gtk::Label>,
        #[template_child]
        pub week_pace: TemplateChild<gtk::Label>,
        #[template_child]
        pub week_progress: TemplateChild<WeekProgress>,
        #[template_child]
        pub week_chart: TemplateChild<WeekChart>,
        #[template_child]
        pub legend: TemplateChild<gtk::FlowBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CalendarView {
        const NAME: &'static str = "BitLogCalendarView";
        type Type = super::CalendarView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            WeekChart::ensure_type();
            WeekProgress::ensure_type();
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
            self.breakpoint_bin
                .connect_current_breakpoint_notify(glib::clone!(
                    #[weak(rename_to = view)]
                    self.obj(),
                    move |bin| {
                        let imp = view.imp();
                        let density = match bin.current_breakpoint() {
                            None => Density::Full,
                            Some(breakpoint) if breakpoint == *imp.short_breakpoint => {
                                Density::Short
                            }
                            Some(_) => Density::Stacked,
                        };
                        if imp.density.replace(density) != density
                            && imp.vault.borrow().is_some()
                            && imp.view_stack.visible_child_name().as_deref() == Some("month")
                        {
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
    /// A month of the vault as a grid of days, or a week as a chart.
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
        let period = if page == "week" {
            self.show_week(date);
            Period::Week
        } else {
            self.show_month(date);
            Period::Month
        };
        // Going to today would change nothing.
        let (first, last) = period.range(date, self.vault().config().week.first_day);
        let today = Local::now().date_naive();
        imp.today_button
            .set_sensitive(!(first..=last).contains(&today));
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

    /// Shows the week `date` lies in as a chart.
    fn show_week_of(&self, date: NaiveDate) {
        self.imp().date.set(date);
        // Showing the week is left to the handler of the toggle.
        self.imp().view_toggle.set_active_name(Some("week"));
    }

    fn show_month(&self, date: NaiveDate) {
        let imp = self.imp();
        let vault = self.vault();
        let vault = &*vault;
        let first_day = vault.config().week.first_day;
        let (first, last) = Period::Month.range(date, first_day);
        imp.window_title.set_title(&format_month(first));
        imp.window_title.set_subtitle(&first.year().to_string());
        self.set_title(&format_month_year(first));

        let month = &imp.month;
        while let Some(child) = month.first_child() {
            month.remove(&child);
        }
        let density = imp.density.get();
        // Keeps the week numbers as wide as the space above them.
        let week_numbers = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);

        let header = gtk::Box::builder().spacing(6).build();
        let corner = gtk::Label::builder()
            // Translators: The heading of the column of week numbers in the
            // calendar, short for "Week".
            .label(gettext("Wk"))
            .css_classes(["caption-heading", "dim-label"])
            .build();
        week_numbers.add_widget(&corner);
        header.append(&corner);
        let weekdays = gtk::Box::builder().homogeneous(true).hexpand(true).build();
        let week_start = week_start(first, first_day);
        for offset in 0..7 {
            let weekday = format_date(week_start + Days::new(offset), "%a");
            weekdays.append(
                &gtk::Label::builder()
                    .label(weekday)
                    .xalign(0.0)
                    .margin_start(8)
                    .ellipsize(pango::EllipsizeMode::End)
                    .css_classes(["caption-heading", "dim-label"])
                    .build(),
            );
        }
        header.append(&weekdays);
        month.append(&header);

        // Both columns are homogeneous and as high, so their rows line up.
        let body = gtk::Box::builder().spacing(6).vexpand(true).build();
        let numbers = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .homogeneous(true)
            .build();
        week_numbers.add_widget(&numbers);
        let weeks = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .homogeneous(true)
            .hexpand(true)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["card", "calendar-month"])
            .build();
        let today = Local::now().date_naive();
        let mut week = week_start;
        // Whole weeks, from the one with the first to the one with the last day.
        while week <= last {
            numbers.append(&self.week_number_button(week));
            let row = gtk::Box::builder()
                .homogeneous(true)
                .css_classes(["calendar-week"])
                .build();
            for offset in 0..7 {
                let date = week + Days::new(offset);
                let in_month = date.month() == first.month();
                row.append(&day_cell(vault, date, in_month, date == today, density));
            }
            weeks.append(&row);
            week = week + Days::new(7);
        }
        body.append(&numbers);
        body.append(&weeks);
        month.append(&body);
    }

    /// The number of the week starting on `week`, which shows it as a chart.
    fn week_number_button(&self, week: NaiveDate) -> gtk::Button {
        let number = week_number(week).to_string();
        let button = gtk::Button::builder()
            .label(&number)
            .valign(gtk::Align::Start)
            .tooltip_text(gettext("Show Week"))
            .css_classes(["flat", "dim-label", "calendar-week-number"])
            .build();
        // Translators: The number of a week of the year, as in "Week 40".
        let name = gettext("Week {number}").replace("{number}", &number);
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.show_week_of(week)
        ));
        button
    }

    fn show_week(&self, date: NaiveDate) {
        let imp = self.imp();
        let vault = self.vault();
        let (first, last) = Period::Week.range(date, vault.config().week.first_day);
        let title = format_range(&format_short_date(first), &format_short_date(last));
        imp.window_title.set_title(&title);
        // Translators: The number of a week and its year, as in "Week 40 · 2026".
        let subtitle = gettext("Week {number} · {year}")
            .replace("{number}", &week_number(first).to_string())
            .replace("{year}", &last.year().to_string());
        imp.window_title.set_subtitle(&subtitle);
        self.set_title(&title);

        let today = Local::now().date_naive();
        let mut worked = TimeDelta::zero();
        let mut per_project = BTreeMap::new();
        let mut days = Vec::new();
        let mut plan = Vec::new();
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
            let usual_hours = vault.config().week.target_hours_on(date.weekday()) as f32;
            // Days off take their share off the target.
            let target_hours = match &day {
                Some(day) if !day.is_work() => 0.0,
                _ => usual_hours,
            };
            let target_time = TimeDelta::minutes((f64::from(target_hours) * 60.0).round() as i64);
            plan.push((date, working_time, target_time));
            for (slug, time) in &times {
                *per_project.entry(slug.clone()).or_insert(TimeDelta::zero()) += *time;
            }
            days.push(ChartDay {
                date,
                is_today: date == today,
                is_workday: vault.config().week.workdays.contains(&date.weekday()),
                kind: day
                    .as_ref()
                    .filter(|day| !day.is_work())
                    .map(|day| kind_name(&day.kind)),
                segments: times
                    .iter()
                    .map(|(slug, time)| (project_color(&vault, slug), hours(*time)))
                    .collect(),
                tooltip: day
                    .as_ref()
                    .and_then(|day| DaySummary::new(&vault, day).details(&vault).tooltip()),
                target_hours: usual_hours,
            });
        }
        imp.week_chart.set_week(days);

        let target: TimeDelta = plan.iter().map(|(.., target)| *target).sum();
        imp.week_total.set_label(&if target > TimeDelta::zero() {
            // Translators: Hours worked in a week against its target, as in
            // "23 h 15 min of 32 h 0 min".
            gettext("{worked} of {target}")
                .replace("{worked}", &format_duration(worked))
                .replace("{target}", &format_duration(target))
        } else {
            format_duration(worked)
        });
        self.show_pace(worked, target, balance(&plan, today));

        imp.legend.remove_all();
        let mut per_project: Vec<_> = per_project.into_iter().collect();
        per_project.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
        for (slug, time) in per_project {
            let name = vault.project_name(&slug);
            let markup = format!(
                "{} {} · {}",
                color_dot(project_hex(&vault, &slug)),
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

    /// The progress of the week towards `target`, with a mark where it
    /// should be by today and how far ahead or behind that it is.
    fn show_pace(
        &self,
        worked: TimeDelta,
        target: TimeDelta,
        balance: Option<(TimeDelta, TimeDelta)>,
    ) {
        let imp = self.imp();
        let expected = balance.map(|(expected, _)| hours(expected));
        imp.week_progress
            .set(hours(worked), hours(target), expected);
        match balance {
            Some((_, balance)) if target > TimeDelta::zero() => {
                imp.week_pace.set_label(&pace_text(balance));
            }
            _ => imp.week_pace.set_label(""),
        }
    }
}

fn hours(time: TimeDelta) -> f32 {
    time.num_minutes() as f32 / 60.0
}

/// The ISO number of the week starting on `week`, that of its Thursday, so
/// weeks starting on another day than Monday get a number too.
fn week_number(week: NaiveDate) -> u32 {
    (0..7)
        .map(|offset| week + Days::new(offset))
        .find(|date| date.weekday() == Weekday::Thu)
        .expect("every week has a Thursday")
        .iso_week()
        .week()
}

/// How far ahead of the target or behind it `balance` is, as in
/// "2 h 15 min behind".
fn pace_text(balance: TimeDelta) -> String {
    let time = format_duration(balance.abs());
    match balance.num_minutes().signum() {
        // Translators: Hours worked in a week short of its target so far, as
        // in "2 h 15 min behind".
        -1 => gettext("{time} behind").replace("{time}", &time),
        // Translators: Hours worked in a week beyond its target so far, as in
        // "1 h 30 min ahead".
        1 => gettext("{time} ahead").replace("{time}", &time),
        // Translators: A week that has met its target so far, to the minute.
        _ => gettext("On track"),
    }
}

/// The target of the days of a week as far as it has come by `today`, and
/// the hours worked against it, `None` for a week still to come. Today
/// counts only as far as it has been worked, so that it is not behind before
/// it is over; hours logged ahead for days to come count as worked. `days`
/// holds the date, the hours worked and the target of each day.
fn balance(
    days: &[(NaiveDate, TimeDelta, TimeDelta)],
    today: NaiveDate,
) -> Option<(TimeDelta, TimeDelta)> {
    let (mut expected, mut worked) = (TimeDelta::zero(), TimeDelta::zero());
    for (date, hours, target) in days {
        worked += *hours;
        if *date < today {
            expected += *target;
        } else if *date == today {
            expected += (*hours).min(*target);
        }
    }
    days.first()
        .is_some_and(|(first, ..)| *first <= today)
        .then(|| (expected, worked - expected))
}

/// A cell for `date` with its hours, projects, location and kind, which
/// opens the day, as much of it as `density` has room for.
fn day_cell(
    vault: &Vault,
    date: NaiveDate,
    in_month: bool,
    is_today: bool,
    density: Density,
) -> gtk::Button {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    let top = gtk::Box::builder().spacing(6).build();
    let number = gtk::Label::builder()
        .label(date.day().to_string())
        .halign(gtk::Align::Start)
        .css_classes(["heading", "numeric", "calendar-day-number"])
        .build();
    if is_today {
        number.add_css_class("today");
    }
    top.append(&number);
    content.append(&top);

    let cell = gtk::Button::builder()
        .child(&content)
        .height_request(112)
        .action_name("win.show-day")
        .action_target(&date.to_string().to_variant())
        .css_classes(["flat", "calendar-day"])
        .build();
    if !in_month {
        cell.add_css_class("dim-label");
    }
    if density != Density::Full {
        cell.add_css_class("narrow");
    }
    if !vault.config().week.workdays.contains(&date.weekday()) {
        cell.add_css_class("non-workday");
    }

    let mut details = Details::default();
    match vault.load_day(date) {
        Ok(Some(DayFile { day, .. })) => {
            let summary = DaySummary::new(vault, &day);
            show_times(vault, &summary, density, &top, &content);
            show_facts(&summary, &cell, &content);
            details = summary.details(vault);
        }
        Ok(None) => {}
        Err(err) => {
            content.append(&cell_label(&gettext("Cannot read"), &["caption", "error"]));
            details.push(
                glib::markup_escape_text(&err.to_string()).into(),
                err.to_string(),
            );
        }
    }

    cell.set_tooltip_markup(details.tooltip().as_deref());
    cell.update_property(&[
        gtk::accessible::Property::Label(&format_full_date(date)),
        gtk::accessible::Property::Description(&details.description.join(", ")),
    ]);
    cell
}

/// Shows the hours of a day in `top`, or below it in `content` when
/// stacked, and its projects with the most time in `content`.
fn show_times(
    vault: &Vault,
    summary: &DaySummary,
    density: Density,
    top: &gtk::Box,
    content: &gtk::Box,
) {
    let DaySummary {
        working_time,
        times,
        ..
    } = summary;
    if !working_time.is_zero() {
        let shown = if density == Density::Full {
            format_duration(*working_time)
        } else {
            format_short_duration(*working_time)
        };
        let hours = cell_label(&shown, &["caption", "numeric"]);
        // The hours are worse to lose than the names below.
        hours.set_ellipsize(pango::EllipsizeMode::None);
        if density == Density::Stacked {
            content.append(&hours);
        } else {
            hours.set_hexpand(true);
            hours.set_xalign(1.0);
            top.append(&hours);
        }
    }
    if !times.is_empty() {
        let total: f32 = times.iter().map(|(_, time)| hours(*time)).sum();
        let parts = times
            .iter()
            .map(|(slug, time)| (project_color(vault, slug), hours(*time) / total))
            .collect();
        let bar = ShareBar::new(parts, 4);
        bar.set_hexpand(true);
        content.append(&bar);
    }
    for (slug, _) in times.iter().take(PROJECTS_SHOWN) {
        let name = glib::markup_escape_text(vault.project_name(slug));
        let dot = color_dot(project_hex(vault, slug));
        let line = cell_label(&format!("{dot} {name}"), &["caption"]);
        line.set_use_markup(true);
        content.append(&line);
    }
    if times.len() > PROJECTS_SHOWN {
        let more = (times.len() - PROJECTS_SHOWN).to_string();
        // Translators: Projects of a day in the calendar that do not
        // fit into it, as in "+2 more".
        let text = gettext("+{count} more").replace("{count}", &more);
        content.append(&cell_label(&text, &["caption", "dim-label"]));
    }
}

/// Shows the location of a day, and its kind unless it is a work day, at
/// the bottom of `content`.
fn show_facts(summary: &DaySummary, cell: &gtk::Button, content: &gtk::Box) {
    if summary.is_day_off {
        cell.add_css_class("day-off");
    }
    if !summary.facts.is_empty() {
        // Pushed to the bottom of the cell.
        let footer = cell_label(&summary.facts.join(" · "), &["caption", "dim-label"]);
        footer.set_vexpand(true);
        footer.set_valign(gtk::Align::End);
        content.append(&footer);
    }
}

fn cell_label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(classes)
        .build()
}

/// How many projects a day in the calendar names, those with the most time.
const PROJECTS_SHOWN: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_counts_today_only_as_far_as_worked() {
        let hours = TimeDelta::hours;
        let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let mut week: Vec<_> = (0..7)
            .map(|offset| {
                let target = if offset < 5 { hours(8) } else { hours(0) };
                let worked = if offset < 3 { hours(7) } else { hours(0) };
                (monday + Days::new(offset), worked, target)
            })
            .collect();
        let wednesday = monday + Days::new(2);
        // Two days short an hour each, today not over yet.
        assert_eq!(balance(&week, wednesday), Some((hours(23), hours(-2))));
        // A meeting logged for Friday already.
        week[4].1 = hours(1);
        assert_eq!(balance(&week, wednesday), Some((hours(23), hours(-1))));
        // A week gone by against all of its target.
        let next_monday = monday + Days::new(7);
        assert_eq!(balance(&week, next_monday), Some((hours(40), hours(-18))));
        assert_eq!(balance(&week, monday - Days::new(1)), None);
    }

    #[test]
    fn weeks_are_numbered_by_their_thursday() {
        let date = |month, day| NaiveDate::from_ymd_opt(2026, month, day).unwrap();
        // From Monday and from Sunday.
        assert_eq!(week_number(date(9, 28)), 40);
        assert_eq!(week_number(date(9, 27)), 40);
        // From Saturday, a week whose Thursday lies in the new year.
        assert_eq!(week_number(date(1, 3)), 2);
        assert_eq!(week_number(date(12, 28)), 53);
    }
}
