//! Time per project and remote work days as the terminal shows them.

use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};
use knotbook_core::{ProjectSlug, Vault};
use knotbook_index::RemoteDays;

use crate::day::format_duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
    Year,
}

/// The first and last day of the `period` that holds `today`. Weeks start
/// on `first_day`.
pub fn current(period: Period, today: NaiveDate, first_day: Weekday) -> (NaiveDate, NaiveDate) {
    let first = match period {
        Period::Week => today - Days::new(today.weekday().days_since(first_day).into()),
        Period::Month => today.with_day(1).expect("every month has a first day"),
        Period::Year => today.with_ordinal(1).expect("every year has a first day"),
    };
    let next = match period {
        Period::Week => first + Days::new(7),
        Period::Month => first + Months::new(1),
        Period::Year => first + Months::new(12),
    };
    (first, next.pred_opt().expect("the day before exists"))
}

/// The period, a table of the time per project without breaks, most
/// first, with each project's share, the total and the breaks, then the
/// remote work days of each year the period touches. `times` holds the time
/// of all projects, as the index sums it up.
pub fn format_stats(
    vault: &Vault,
    first: NaiveDate,
    last: NaiveDate,
    times: &[(ProjectSlug, TimeDelta)],
    remote: &[RemoteDays],
) -> String {
    let mut lines = vec![format!("{first} – {last}"), String::new()];
    let is_break = |slug: &ProjectSlug| vault.project(slug).is_some_and(|p| p.is_break());
    let (breaks, work): (Vec<_>, Vec<_>) = times.iter().partition(|(slug, _)| is_break(slug));
    let total: TimeDelta = work.iter().map(|(_, time)| *time).sum();
    if work.is_empty() {
        lines.push("No blocks.".to_owned());
    } else {
        let mut rows: Vec<[String; 3]> = work
            .iter()
            .map(|(slug, time)| {
                let name = vault
                    .project(slug)
                    .map_or(slug.as_str(), |project| &project.name);
                let total = total.num_minutes().max(1);
                let share = (time.num_minutes() * 100 + total / 2) / total;
                [
                    name.to_owned(),
                    format_duration(*time),
                    format!("{share} %"),
                ]
            })
            .collect();
        rows.push(["Total".to_owned(), format_duration(total), String::new()]);
        let breaks: TimeDelta = breaks.iter().map(|(_, time)| *time).sum();
        if !breaks.is_zero() {
            rows.push(["Breaks".to_owned(), format_duration(breaks), String::new()]);
        }
        let width = |column: usize| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        };
        let (name_width, time_width, share_width) = (width(0), width(1), width(2));
        for [name, time, share] in &rows {
            let line = format!("{name:<name_width$}  {time:>time_width$}  {share:>share_width$}");
            lines.push(line.trim_end().to_owned());
        }
    }
    lines.push(String::new());
    for year in first.year()..=last.year() {
        let days = remote.iter().find(|days| days.year == year);
        let (remote, hybrid) = days.map_or((0, 0), |days| (days.remote, days.hybrid));
        lines.push(format!(
            "Remote work days in {year}: {remote} remote, {hybrid} hybrid"
        ));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn current_periods() {
        // A Wednesday.
        let today = date(2026, 9, 23);
        assert_eq!(
            current(Period::Week, today, Weekday::Mon),
            (date(2026, 9, 21), date(2026, 9, 27))
        );
        assert_eq!(
            current(Period::Week, today, Weekday::Sun),
            (date(2026, 9, 20), date(2026, 9, 26))
        );
        assert_eq!(
            current(Period::Week, date(2026, 9, 21), Weekday::Mon).0,
            date(2026, 9, 21)
        );
        assert_eq!(
            current(Period::Month, today, Weekday::Mon),
            (date(2026, 9, 1), date(2026, 9, 30))
        );
        assert_eq!(
            current(Period::Month, date(2028, 2, 10), Weekday::Mon),
            (date(2028, 2, 1), date(2028, 2, 29))
        );
        assert_eq!(
            current(Period::Year, today, Weekday::Mon),
            (date(2026, 1, 1), date(2026, 12, 31))
        );
    }

    #[test]
    fn stats_table() {
        let vault =
            Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
                .unwrap();
        let time = |slug: &str, minutes| (slug.parse().unwrap(), TimeDelta::minutes(minutes));
        let times = [
            time("infra", 450),
            time("webshop", 300),
            time("pause", 90),
            time("gone", 30),
        ];
        let remote = [RemoteDays {
            year: 2026,
            remote: 1,
            hybrid: 1,
        }];
        let first = date(2025, 12, 29);
        let expected = "\
2025-12-29 – 2026-09-30

Infrastructure  7 h 30 min  58 %
Webshop                5 h  38 %
gone                30 min   4 %
Total                 13 h
Breaks          1 h 30 min

Remote work days in 2025: 0 remote, 0 hybrid
Remote work days in 2026: 1 remote, 1 hybrid
";
        let stats = format_stats(&vault, first, date(2026, 9, 30), &times, &remote);
        assert_eq!(stats, expected);
        let empty = format_stats(&vault, first, first, &[], &[]);
        assert!(empty.contains("No blocks.\n"), "{empty}");
    }
}
