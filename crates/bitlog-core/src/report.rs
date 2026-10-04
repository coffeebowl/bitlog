//! A report of a week for the exports folder.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::Write;

use chrono::{Days, NaiveDate, TimeDelta};

use crate::Vault;
use crate::error::ReadError;

impl Vault {
    /// A report of the week starting on `first` in Markdown: the time per
    /// project, breaks left out, then each day with its details, day note and
    /// blocks with their texts. Times are written as hours and minutes, `7:45`.
    pub fn week_report(&self, first: NaiveDate) -> Result<String, ReadError> {
        let last = first + Days::new(6);
        let mut days = Vec::new();
        for date in self.days(first, last)? {
            if let Some(file) = self.load_day(date)? {
                days.push(file.day);
            }
        }
        let mut times = BTreeMap::new();
        for day in &days {
            for (slug, time) in day.time_per_project(self.projects()) {
                *times.entry(slug).or_insert(TimeDelta::zero()) += time;
            }
        }
        let mut times: Vec<_> = times.into_iter().collect();
        times.sort_by_key(|(_, time)| Reverse(*time));
        let name = |slug| self.project_name(slug);

        let mut report = format!("# Week of {first}\n\n{first} – {last}\n\n");
        if times.is_empty() {
            report.push_str("Nothing logged.\n");
        } else {
            report.push_str("| Project | Time |\n| --- | ---: |\n");
            for (slug, time) in &times {
                let _ = writeln!(report, "| {} | {} |", table_cell(name(slug)), hours(*time));
            }
            let total = times.iter().map(|(_, time)| *time).sum();
            let _ = writeln!(report, "| **Total** | **{}** |", hours(total));
        }
        for day in &days {
            // One blank line before, whatever the day before ended with.
            report.truncate(report.trim_end().len());
            let _ = write!(report, "\n\n## {}\n\n", day.date.format("%A, %Y-%m-%d"));
            let mut details = vec![capitalize(&day.kind)];
            if let Some(key) = &day.location {
                details.push(self.config().location_name(key).to_owned());
            }
            details.push(format!(
                "{} worked",
                hours(day.working_time(self.projects()))
            ));
            let _ = writeln!(report, "{}", details.join(" · "));
            if !day.note.is_empty() {
                let _ = write!(report, "\n{}\n", day.note);
            }
            if !day.blocks.is_empty() {
                report.push('\n');
            }
            for block in &day.blocks {
                let mut line = format!("- **{}** {}", block.times(), name(&block.project));
                if !block.title.is_empty() {
                    let _ = write!(line, ": {}", block.title);
                }
                let _ = writeln!(report, "{line}");
                if !block.text.is_empty() {
                    // Indented, so that the text belongs to the list item.
                    report.push('\n');
                    for text_line in block.text.lines() {
                        if text_line.is_empty() {
                            report.push('\n');
                        } else {
                            let _ = writeln!(report, "  {text_line}");
                        }
                    }
                    report.push('\n');
                }
            }
        }
        report.truncate(report.trim_end().len());
        report.push('\n');
        Ok(report)
    }
}

/// A duration as hours and minutes, as in `7:45`.
fn hours(time: TimeDelta) -> String {
    let minutes = time.num_minutes();
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// `text` for a cell of a Markdown table.
fn table_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// `kind` is free text, usually lowercase like `work`.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::sample_copy;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    #[test]
    fn week_as_markdown() {
        let (_dir, vault) = sample_copy();
        let report = vault.week_report(date(21)).unwrap();
        assert!(
            report.starts_with("# Week of 2026-09-21\n\n2026-09-21 – 2026-09-27\n\n"),
            "{report}"
        );
        assert!(report.contains("| Webshop | 10:45 |\n"), "{report}");
        assert!(report.contains("| **Total** | **22:15** |\n"), "{report}");
        assert!(
            report.contains("## Monday, 2026-09-21\n\nWork · Remote · 7:45 worked\n"),
            "{report}"
        );
        assert!(
            report.contains("- **22:30–00:30** Infrastructure: Release deployment\n\n  Deployed"),
            "{report}"
        );
        assert!(
            !report.contains("| Break |"),
            "breaks are left out of the table"
        );
        let empty = vault.week_report(date(28)).unwrap();
        assert!(empty.ends_with("Nothing logged.\n"), "{empty}");
        assert!(!report.contains("\n\n\n"), "{report}");
        assert!(report.ends_with('\n') && !report.ends_with("\n\n"));
    }
}
