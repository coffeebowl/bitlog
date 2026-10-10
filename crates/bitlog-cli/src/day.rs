//! Days as the terminal shows them.

use bitlog_core::{Day, Vault, format_span};
use chrono::{NaiveDate, TimeDelta};

use crate::table::table;

/// The heading of the day `date`, as in "Wednesday, 2026-09-23".
pub fn heading(date: NaiveDate) -> String {
    date.format("%A, %Y-%m-%d").to_string()
}

/// The heading, the day's details and a table of its blocks with their ids.
pub fn format_day(vault: &Vault, day: &Day) -> String {
    let details = vault.day_details(day, format_duration);
    let mut text = format!("{}\n{details}\n\n", heading(day.date));
    if day.blocks.is_empty() {
        text.push_str("No blocks.\n");
    }
    let rows: Vec<[String; 5]> = day
        .blocks
        .iter()
        .map(|block| {
            let project = vault.project_name(&block.project);
            [
                block.id.to_string(),
                format_span(block.span()),
                format_duration(block.duration()),
                project.to_owned(),
                block.title.clone(),
            ]
        })
        .collect();
    text.push_str(&table(&rows, &[2]));
    text
}

/// "7 h 45 min", leaving out parts that are zero.
pub fn format_duration(duration: TimeDelta) -> String {
    let total = duration.num_minutes();
    match (total / 60, total % 60) {
        (hours, 0) => format!("{hours} h"),
        (0, minutes) => format!("{minutes} min"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
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
             aa11  09:30–10:00        30 min  Filler          Mails\n\
             bb22  10:00–10:15        15 min  Meetings        Daily\n\
             cc33  10:15–12:30    2 h 15 min  Infrastructure  Prepare release deployment\n\
             dd44  12:30–13:15        45 min  Break           Lunch\n\
             ee55  13:15–15:00    1 h 45 min  Webshop         Release notes\n\
             ff66  22:30–00:30+1         2 h  Infrastructure  Release deployment\n"
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
