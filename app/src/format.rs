//! Dates, times and durations as the app shows them.

use chrono::{Datelike, NaiveDate, NaiveTime, TimeDelta};
use gettextrs::gettext;
use gtk::glib;

/// `date` formatted with the codes of g_date_time_format(), in the user's
/// language.
pub fn format_date(date: NaiveDate, format: &str) -> String {
    glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    )
    .expect("every date chrono knows is valid in GLib")
    .format(format)
    .expect("the date format is valid")
    .to_string()
}

/// A date without the weekday, as in "September 22, 2026".
pub fn format_full_date(date: NaiveDate) -> String {
    // Translators: A date without the weekday, as in "September 22, 2026".
    // See the GLib documentation of g_date_time_format() for the codes.
    format_date(date, &gettext("%B %-d, %Y"))
}

pub fn format_time(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
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

/// `kind` is free text in the file, usually lowercase like `work`.
pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
