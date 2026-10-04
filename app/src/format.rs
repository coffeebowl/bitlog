//! Dates, times, durations and counts as the app shows them.

use bitlog_core::ProjectStatus;
use chrono::{Datelike, NaiveDate, NaiveTime, TimeDelta};
use gettextrs::gettext;
use gtk::glib;

/// `count` for choosing a plural form with `ngettext`.
pub fn plural(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The title of a block as Pango markup. Without a title, the name of its
/// project stands in for it, in italics, as in the day's timeline.
pub fn title_markup(title: &str, project_name: &str) -> String {
    if title.is_empty() {
        format!("<i>{}</i>", glib::markup_escape_text(project_name))
    } else {
        glib::markup_escape_text(title).to_string()
    }
}

/// `date` formatted with the codes of g_date_time_format(), in the user's
/// language.
pub fn format_date(date: NaiveDate, format: &str) -> String {
    glib_date(date)
        .format(format)
        .expect("the date format is valid")
        .to_string()
}

/// The start of `date` in local time, as GTK takes dates.
pub fn glib_date(date: NaiveDate) -> glib::DateTime {
    glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    )
    .expect("every date chrono knows is valid in GLib")
}

/// The date of `date`, a date as GTK gives it.
pub fn naive_date(date: &glib::DateTime) -> NaiveDate {
    NaiveDate::from_ymd_opt(
        date.year(),
        date.month().unsigned_abs(),
        date.day_of_month().unsigned_abs(),
    )
    .expect("GLib dates are valid")
}

/// A date without the year, as in "Sep 30".
pub fn format_short_date(date: NaiveDate) -> String {
    // Translators: A date without the year, as in "Sep 30". See the GLib
    // documentation of g_date_time_format() for the codes.
    format_date(date, &gettext("%b %-d"))
}

/// A date without the weekday, as in "September 22, 2026".
pub fn format_full_date(date: NaiveDate) -> String {
    // Translators: A date without the weekday, as in "September 22, 2026".
    // See the GLib documentation of g_date_time_format() for the codes.
    format_date(date, &gettext("%B %-d, %Y"))
}

/// `date` without the year if it is the year of `today`, as in "Sep 22",
/// else as in "September 22, 2025".
pub fn format_recent_date(date: NaiveDate, today: NaiveDate) -> String {
    if date.year() == today.year() {
        format_short_date(date)
    } else {
        format_full_date(date)
    }
}

/// "Today", "Yesterday", or `date` as `format_recent_date` writes it, as
/// seen `today`.
pub fn format_relative_day(date: NaiveDate, today: NaiveDate) -> String {
    if date == today {
        gettext("Today")
    } else if today.pred_opt() == Some(date) {
        gettext("Yesterday")
    } else {
        format_recent_date(date, today)
    }
}

/// The month of `date`, as in "September".
pub fn format_month(date: NaiveDate) -> String {
    // Translators: The name of a month, as in "September". See the GLib
    // documentation of g_date_time_format() for the codes.
    format_date(date, &gettext("%B"))
}

/// The month of `date` and its year, as in "September 2026".
pub fn format_month_year(date: NaiveDate) -> String {
    // Translators: A month and its year, as in "September 2026". See the
    // GLib documentation of g_date_time_format() for the codes.
    format_date(date, &gettext("%B %Y"))
}

/// From the date `first` to the date `last`, both formatted, as in
/// "Sep 21 – Sep 27".
pub fn format_range(first: &str, last: &str) -> String {
    // Translators: A range of dates, as in "Sep 21 – Sep 27".
    gettext("{first} – {last}")
        .replace("{first}", first)
        .replace("{last}", last)
}

/// The weekday and day of the month of `date`, where there is little
/// room, as in "Mon 21".
pub fn format_weekday_day(date: NaiveDate) -> String {
    // Translators: A short weekday and the day of the month, as in
    // "Mon 21". See the GLib documentation of g_date_time_format() for the
    // codes.
    format_date(date, &gettext("%a %-d"))
}

/// A date with the weekday, as in "Tue, Sep 22", and the year unless it is
/// the year of `today`.
pub fn format_weekday_date(date: NaiveDate, today: NaiveDate) -> String {
    let format = if date.year() == today.year() {
        // Translators: A date with the weekday, as in "Tue, Sep 22". See the
        // GLib documentation of g_date_time_format() for the codes.
        gettext("%a, %b %-d")
    } else {
        // Translators: A date with the weekday and year, as in
        // "Tue, Sep 22, 2026". See the GLib documentation of
        // g_date_time_format() for the codes.
        gettext("%a, %b %-d, %Y")
    };
    format_date(date, &format)
}

pub fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

/// From `start` to `end`, as in "09:00–10:30".
pub fn format_span(start: NaiveTime, end: NaiveTime) -> String {
    format!("{}–{}", format_time(start), format_time(end))
}

pub fn format_duration(duration: TimeDelta) -> String {
    let total = duration.num_minutes();
    let (hours, minutes) = (total / 60, total % 60);
    let text = match (hours, minutes) {
        // Translators: A duration of full hours, as in "8 h".
        (_, 0) => gettext("{hours} h"),
        // Translators: A duration below an hour, as in "45 min".
        (0, _) => gettext("{minutes} min"),
        // Translators: A duration, as in "7 h 45 min".
        _ => gettext("{hours} h {minutes} min"),
    };
    text.replace("{hours}", &hours.to_string())
        .replace("{minutes}", &minutes.to_string())
}

/// `duration` where there is little room, as in "7:45".
pub fn format_short_duration(duration: TimeDelta) -> String {
    let total = duration.num_minutes();
    format!("{}:{:02}", total / 60, total % 60)
}

/// `part` as a share of `whole`, rounded to whole percent, as in "45 %".
/// `whole` must not be zero.
pub fn format_share(part: TimeDelta, whole: TimeDelta) -> String {
    let whole = whole.num_minutes();
    let share = (part.num_minutes() * 100 + whole / 2) / whole;
    // Translators: A share in percent, as in "45 %".
    gettext("{share} %").replace("{share}", &share.to_string())
}

/// The kinds of day the app offers. Files may have others, which are kept.
pub const DAY_KINDS: [&str; 4] = ["work", "vacation", "sick", "holiday"];

/// The name of the kind of day `kind`, which is free text in the file.
pub fn kind_name(kind: &str) -> String {
    match kind {
        "work" => gettext("Work"),
        "vacation" => gettext("Vacation"),
        "sick" => gettext("Sick"),
        "holiday" => gettext("Public Holiday"),
        _ => capitalize(kind),
    }
}

/// The project statuses in the order the app lists them.
pub const PROJECT_STATUSES: [ProjectStatus; 3] = [
    ProjectStatus::Active,
    ProjectStatus::Paused,
    ProjectStatus::Archived,
];

pub fn status_name(status: ProjectStatus) -> String {
    match status {
        ProjectStatus::Active => gettext("Active"),
        ProjectStatus::Paused => gettext("Paused"),
        ProjectStatus::Archived => gettext("Archived"),
    }
}

/// `text` with a capital first letter, as kinds of day and categories are
/// usually written in lowercase.
pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stand_in_titles_are_italic() {
        assert_eq!(title_markup("Fix <b>", "Webshop"), "Fix &lt;b&gt;");
        assert_eq!(title_markup("", "R&D"), "<i>R&amp;D</i>");
    }

    #[test]
    fn relative_days() {
        let date = |year, month, day| NaiveDate::from_ymd_opt(year, month, day).unwrap();
        let today = date(2026, 10, 4);
        assert_eq!(format_relative_day(today, today), "Today");
        assert_eq!(format_relative_day(date(2026, 10, 3), today), "Yesterday");
        // The year only for other years.
        assert!(!format_relative_day(date(2026, 9, 22), today).contains("2026"));
        assert!(format_relative_day(date(2025, 9, 22), today).contains("2025"));
        assert!(format_recent_date(date(2025, 12, 31), date(2026, 1, 1)).contains("2025"));
    }

    #[test]
    fn plural_counts_fit() {
        assert_eq!(plural(2), 2);
        assert_eq!(plural(usize::MAX), u32::MAX);
    }
}
