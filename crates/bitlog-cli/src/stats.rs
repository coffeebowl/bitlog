//! Time per project and remote work days as the terminal shows them.

use bitlog_core::{ProjectCategory, ProjectSlug, Vault, percent};
use bitlog_index::RemoteDays;
use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::day::format_duration;
use crate::table::table;

/// The period, a table of the time per project that counts as working time,
/// most first, with each project's share, the total, the private time and the
/// breaks, then the remote work days of each year the period touches. `times`
/// holds the time of all projects, as the index sums it up.
pub fn format_stats(
    vault: &Vault,
    first: NaiveDate,
    last: NaiveDate,
    times: &[(ProjectSlug, TimeDelta)],
    remote: &[RemoteDays],
) -> String {
    let mut lines = vec![format!("{first} – {last}"), String::new()];
    let work: Vec<_> = times
        .iter()
        .filter(|(slug, _)| vault.project_category(slug).counts_as_working_time())
        .collect();
    let total: TimeDelta = work.iter().map(|(_, time)| *time).sum();
    if work.is_empty() {
        lines.push("No blocks.".to_owned());
    } else {
        let mut rows: Vec<[String; 3]> = work
            .iter()
            .map(|(slug, time)| {
                let name = vault.project_name(slug);
                [
                    name.to_owned(),
                    format_duration(*time),
                    format!("{} %", percent(*time, total)),
                ]
            })
            .collect();
        rows.push(["Total".to_owned(), format_duration(total), String::new()]);
        for (name, category) in [
            ("Private", ProjectCategory::Private),
            ("Breaks", ProjectCategory::Break),
        ] {
            let time: TimeDelta = times
                .iter()
                .filter(|(slug, _)| vault.project_category(slug) == category)
                .map(|(_, time)| *time)
                .sum();
            if !time.is_zero() {
                rows.push([name.to_owned(), format_duration(time), String::new()]);
            }
        }
        lines.push(table(&rows, &[1, 2]).trim_end().to_owned());
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

    #[test]
    fn private_time_stands_below_the_total() {
        let root = std::env::temp_dir().join(format!("bitlog-stats-{}", fastrand::u64(..)));
        let vault = Vault::create(&root, "Test", date(2026, 3, 1)).unwrap();
        let time = |slug: &str, minutes| (slug.parse().unwrap(), TimeDelta::minutes(minutes));
        let times = [
            time("company", 360),
            time("side-project", 120),
            time("filler", 120),
            time("pause", 30),
        ];
        let first = date(2026, 3, 2);
        let stats = format_stats(&vault, first, first, &times, &[]);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            stats.contains(
                "Your Company     6 h  75 %\n\
                 Filler           2 h  25 %\n\
                 Total            8 h\n\
                 Private          2 h\n\
                 Breaks        30 min\n"
            ),
            "{stats}"
        );
    }
}
