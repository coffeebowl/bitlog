//! Days as the terminal shows them.

use chrono::{NaiveDate, NaiveTime, TimeDelta};
use knotbook_core::{Day, Vault};

/// The heading of the day `date`, as in "Wednesday, 2026-09-23".
pub fn heading(date: NaiveDate) -> String {
    date.format("%A, %Y-%m-%d").to_string()
}

/// The heading, the day's details and a table of its blocks.
pub fn format_day(vault: &Vault, day: &Day) -> String {
    let mut details = vec![capitalize(&day.kind)];
    if let Some(key) = &day.location {
        details.push(vault.config().location_name(key).to_owned());
    }
    let mut working_time = format_duration(day.working_time(vault.projects())) + " worked";
    if let (Some(start), Some(end)) = (day.work_start, day.work_end) {
        working_time += &format!(" ({})", format_span(start, end));
    }
    details.push(working_time);

    let mut lines = vec![heading(day.date), details.join(" · "), String::new()];
    if day.blocks.is_empty() {
        lines.push("No blocks.".to_owned());
    }
    let rows: Vec<[String; 4]> = day
        .blocks
        .iter()
        .map(|block| {
            let project = vault
                .project(&block.project)
                .map_or(block.project.as_str(), |project| &project.name);
            [
                format_span(block.start, block.end),
                format_duration(block.duration()),
                project.to_owned(),
                block.title.clone(),
            ]
        })
        .collect();
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (span_width, duration_width, project_width) = (width(0), width(1), width(2));
    for [span, duration, project, title] in &rows {
        let line = format!(
            "{span:<span_width$}  {duration:>duration_width$}  {project:<project_width$}  {title}"
        );
        lines.push(line.trim_end().to_owned());
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// "08:00–16:30", with "+1" for an end on the next day.
fn format_span(start: NaiveTime, end: NaiveTime) -> String {
    let next_day = if end <= start { "+1" } else { "" };
    format!(
        "{}–{}{next_day}",
        start.format("%H:%M"),
        end.format("%H:%M")
    )
}

/// "7 h 45 min", leaving out parts that are zero.
fn format_duration(duration: TimeDelta) -> String {
    let total = duration.num_minutes();
    match (total / 60, total % 60) {
        (hours, 0) => format!("{hours} h"),
        (0, minutes) => format!("{minutes} min"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// `kind` is free text in the file, usually lowercase like `work`.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn sample(day: u32) -> String {
        let vault =
            Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
                .unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        format_day(&vault, &vault.load_day(date).unwrap().unwrap().day)
    }

    #[test]
    fn day_with_blocks_past_midnight() {
        assert_eq!(
            sample(23),
            "Wednesday, 2026-09-23\n\
             Work · Hybrid · 6 h 45 min worked\n\
             \n\
             09:30–10:00        30 min  Filler          Mails\n\
             10:00–10:15        15 min  Meetings        Daily\n\
             10:15–12:30    2 h 15 min  Infrastructure  Prepare release deployment\n\
             12:30–13:15        45 min  Break           Lunch\n\
             13:15–15:00    1 h 45 min  Webshop         Release notes\n\
             22:30–00:30+1         2 h  Infrastructure  Release deployment\n"
        );
    }

    #[test]
    fn day_with_working_hours() {
        let text = sample(21);
        assert!(
            text.contains("\nWork · Remote · 7 h 45 min worked (08:00–16:30)\n"),
            "{text}"
        );
    }

    #[test]
    fn empty_day() {
        let vault =
            Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
                .unwrap();
        let day = Day::new(NaiveDate::from_ymd_opt(2026, 9, 24).unwrap());
        assert_eq!(
            format_day(&vault, &day),
            "Thursday, 2026-09-24\nWork · 0 h worked\n\nNo blocks.\n"
        );
    }
}
