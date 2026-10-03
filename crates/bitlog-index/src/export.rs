//! Exports of a vault for spreadsheets and reports, written to its exports
//! folder with [`Vault::write_export`].

use std::fmt::Write;

use bitlog_core::{ReadError, Vault};
use chrono::{Days, NaiveDate, TimeDelta};

use crate::{Index, IndexError};

/// The name of the export of the remote work days.
pub const REMOTE_DAYS_FILE: &str = "remote-days.csv";

/// The name of the export of the blocks from `period.0` to `period.1`, or of
/// all blocks.
pub fn blocks_file_name(period: Option<(NaiveDate, NaiveDate)>) -> String {
    match period {
        Some((first, last)) => format!("blocks-{first}-{last}.csv"),
        None => "blocks.csv".to_owned(),
    }
}

/// The name of the report of the week starting on `first`.
pub fn week_file_name(first: NaiveDate) -> String {
    format!("week-{first}.md")
}

impl Index {
    /// The blocks from `period.0` to `period.1`, or all blocks, as CSV
    /// (RFC 4180) with a header, oldest first. The end of a block that ends
    /// on the next day is earlier than its start, as in the day files.
    pub fn blocks_csv(
        &self,
        vault: &Vault,
        period: Option<(NaiveDate, NaiveDate)>,
    ) -> Result<String, IndexError> {
        let blocks = self.blocks_between(period.map(|p| p.0), period.map(|p| p.1))?;
        let mut csv =
            String::from("date,start,end,minutes,project,project_name,category,title,text\r\n");
        for (slug, block) in blocks {
            let project = vault.project(&slug);
            let fields = [
                block.date.to_string(),
                clock(block.start_minute),
                clock(block.end_minute),
                block.duration().num_minutes().to_string(),
                slug.to_string(),
                vault.project_name(&slug).to_owned(),
                project.map_or(String::new(), |project| project.category.clone()),
                block.title,
                block.text,
            ];
            let fields: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
            csv.push_str(&fields.join(","));
            csv.push_str("\r\n");
        }
        Ok(csv)
    }

    /// The remote and hybrid work days of each year as CSV (RFC 4180) with
    /// a header, see [`Index::remote_days`].
    pub fn remote_days_csv(&self) -> Result<String, IndexError> {
        let mut csv = String::from("year,remote,hybrid\r\n");
        for days in self.remote_days()? {
            let _ = write!(csv, "{},{},{}\r\n", days.year, days.remote, days.hybrid);
        }
        Ok(csv)
    }
}

/// A report of the week starting on `first` in Markdown: the time per
/// project, breaks left out, then each day with its details, day note and
/// blocks with their texts. Times are written as hours and minutes, `7:45`.
pub fn week_report(vault: &Vault, first: NaiveDate) -> Result<String, ReadError> {
    let last = first + Days::new(6);
    let mut days = Vec::new();
    for date in vault.days(first, last)? {
        if let Some(file) = vault.load_day(date)? {
            days.push(file.day);
        }
    }
    let mut times = std::collections::BTreeMap::new();
    for day in &days {
        for (slug, time) in day.time_per_project(vault.projects()) {
            *times.entry(slug).or_insert(TimeDelta::zero()) += time;
        }
    }
    let mut times: Vec<_> = times.into_iter().collect();
    times.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
    let name = |slug| vault.project_name(slug);

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
            details.push(vault.config().location_name(key).to_owned());
        }
        details.push(format!(
            "{} worked",
            hours(day.working_time(vault.projects()))
        ));
        let _ = writeln!(report, "{}", details.join(" · "));
        if !day.note.is_empty() {
            let _ = write!(report, "\n{}\n", day.note);
        }
        if !day.blocks.is_empty() {
            report.push('\n');
        }
        for block in &day.blocks {
            let span = format!(
                "{}–{}",
                block.start.format("%H:%M"),
                block.end.format("%H:%M")
            );
            let mut line = format!("- **{span}** {}", name(&block.project));
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

/// `HH:MM` of a minute of the day, which may lie on the next day.
fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// A duration as hours and minutes, as in `7:45`.
fn hours(time: TimeDelta) -> String {
    let minutes = time.num_minutes();
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// `value` as a CSV field, quoted if needed.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
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
    use std::path::Path;

    use super::*;

    fn sample() -> Vault {
        Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
            .unwrap()
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    fn index() -> Index {
        let mut index =
            Index::with_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        index.refresh(&sample()).unwrap();
        index
    }

    #[test]
    fn csv_fields() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a, b"), "\"a, b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn blocks_as_csv() {
        let csv = index()
            .blocks_csv(&sample(), Some((date(23), date(23))))
            .unwrap();
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(
            lines[0],
            "date,start,end,minutes,project,project_name,category,title,text"
        );
        assert_eq!(
            lines.last(),
            Some(&""),
            "every record ends with a line break"
        );
        assert!(
            lines.contains(
                &"2026-09-23,22:30,00:30,120,infra,Infrastructure,work,Release deployment,\
                  \"Deployed 2.4.0 to production, cache purged at 00:10.\""
            ),
            "{csv}"
        );
        // Six blocks, the header and the empty rest after the last record.
        assert_eq!(lines.len(), 8);
        let all = index().blocks_csv(&sample(), None).unwrap();
        assert_eq!(all.matches("\r\n2026-09-").count(), 17);
    }

    #[test]
    fn remote_days_as_csv() {
        assert_eq!(
            index().remote_days_csv().unwrap(),
            "year,remote,hybrid\r\n2026,1,1\r\n"
        );
    }

    #[test]
    fn week_as_markdown() {
        let report = week_report(&sample(), date(21)).unwrap();
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
        let empty = week_report(&sample(), date(28)).unwrap();
        assert!(empty.ends_with("Nothing logged.\n"), "{empty}");
        assert!(!report.contains("\n\n\n"), "{report}");
        assert!(report.ends_with('\n') && !report.ends_with("\n\n"));
    }
}
