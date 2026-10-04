//! Exports of a vault for spreadsheets, and the names of all exports, written
//! to its exports folder with [`Vault::write_export`].

use std::fmt::Write;

use bitlog_core::Vault;
use chrono::NaiveDate;

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

/// `HH:MM` of a minute of the day, which may lie on the next day.
fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// `value` as a CSV field, quoted if needed.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
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
}
