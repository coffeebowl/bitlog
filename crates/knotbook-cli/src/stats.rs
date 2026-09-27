//! Time per project and remote work days as the terminal shows them.

use chrono::{Datelike, NaiveDate, TimeDelta};
use knotbook_core::{ProjectSlug, Vault};
use knotbook_index::RemoteDays;

use crate::day::format_duration;

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
