//! Read access to a whole vault.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Months, NaiveDate};

use crate::error::ReadError;
use crate::{Day, DayWarning, Project, ProjectSlug, VaultConfig};

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
    config: VaultConfig,
    /// Sorted by slug.
    projects: Vec<Project>,
}

impl Vault {
    /// Opens the vault in the folder `root`, reading its configuration and
    /// projects. Days are read on demand.
    pub fn open(root: &Path) -> Result<Self, ReadError> {
        Ok(Self {
            root: root.to_owned(),
            config: VaultConfig::load(&root.join("knotbook.toml"))?,
            projects: Project::load_all(root)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &VaultConfig {
        &self.config
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn project(&self, slug: &ProjectSlug) -> Option<&Project> {
        self.projects.iter().find(|project| project.slug == *slug)
    }

    /// Where the file of the day `date` lives.
    pub fn day_path(&self, date: NaiveDate) -> PathBuf {
        self.root
            .join("daily")
            .join(format!("{:04}", date.year()))
            .join(format!("{:02}", date.month()))
            .join(format!("{}.md", date.format("%Y-%m-%d")))
    }

    /// Reads the day `date`, or returns `None` if there is no file for it.
    pub fn load_day(&self, date: NaiveDate) -> Result<Option<(Day, Vec<DayWarning>)>, ReadError> {
        let path = self.day_path(date);
        if !path.is_file() {
            return Ok(None);
        }
        Day::load(&path).map(Some)
    }

    /// The dates from `first` to `last`, both included, that have a day file.
    pub fn days(&self, first: NaiveDate, last: NaiveDate) -> Result<Vec<NaiveDate>, ReadError> {
        let mut dates = Vec::new();
        let mut month = first.with_day(1).expect("every month has a first day");
        while month <= last {
            let folder = self
                .day_path(month)
                .parent()
                .expect("day files lie in a folder")
                .to_owned();
            dates.extend(
                day_files(&folder)?
                    .into_iter()
                    .filter(|date| (first..=last).contains(date)),
            );
            month = month + Months::new(1);
        }
        dates.sort();
        Ok(dates)
    }
}

/// The dates of the day files in the month folder `folder`.
///
/// Only names of the exact form `YYYY-MM-DD.md` of that month count, which
/// leaves out sync conflict copies and any other file.
fn day_files(folder: &Path) -> Result<Vec<NaiveDate>, ReadError> {
    let io_error = |source| ReadError::Io {
        path: folder.to_owned(),
        source,
    };
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io_error(err)),
    };
    let mut dates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let Some(stem) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".md"))
            .map(str::to_owned)
        else {
            continue;
        };
        // Parsing alone would also accept `2026-9-1`.
        let Some(date) = NaiveDate::parse_from_str(&stem, "%Y-%m-%d")
            .ok()
            .filter(|date| date.format("%Y-%m-%d").to_string() == stem)
        else {
            continue;
        };
        if folder.ends_with(format!("{:04}/{:02}", date.year(), date.month())) {
            dates.push(date);
        }
    }
    Ok(dates)
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    fn sample_vault() -> Vault {
        Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
            .unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn open_sample_vault() {
        let vault = sample_vault();
        assert_eq!(vault.config().name, "Sample Knotbook");
        assert_eq!(vault.projects().len(), 5);
        let webshop = vault.project(&"webshop".parse().unwrap()).unwrap();
        assert_eq!(webshop.name, "Webshop");
        assert!(vault.project(&"unknown".parse().unwrap()).is_none());
    }

    #[test]
    fn days_ignore_conflict_copies() {
        let vault = sample_vault();
        let all = vault.days(date(2020, 1, 1), date(2030, 12, 31)).unwrap();
        assert_eq!(
            all,
            [date(2026, 9, 21), date(2026, 9, 22), date(2026, 9, 23)]
        );
        let some = vault.days(date(2026, 9, 22), date(2026, 9, 22)).unwrap();
        assert_eq!(some, [date(2026, 9, 22)]);
        assert!(
            vault
                .days(date(2026, 9, 24), date(2026, 9, 20))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn load_day() {
        let vault = sample_vault();
        let (day, _) = vault.load_day(date(2026, 9, 22)).unwrap().unwrap();
        assert_eq!(day.note, "Office day, lots of reviews.");
        assert!(vault.load_day(date(2026, 9, 24)).unwrap().is_none());
    }

    #[test]
    fn working_time() {
        let vault = sample_vault();
        let hours = |day: u32| {
            let (day, _) = vault.load_day(date(2026, 9, day)).unwrap().unwrap();
            day.working_time(vault.projects())
        };
        // 08:00 to 16:30 minus a 45 minute break.
        assert_eq!(hours(21), TimeDelta::minutes(7 * 60 + 45));
        // 08:45 to 17:15 minus a 45 minute break.
        assert_eq!(hours(22), TimeDelta::minutes(7 * 60 + 45));
        // No working hours given: all blocks but the break, one past midnight.
        assert_eq!(hours(23), TimeDelta::minutes(30 + 15 + 135 + 105 + 120));
    }

    #[test]
    fn time_per_project() {
        let vault = sample_vault();
        let (day, _) = vault.load_day(date(2026, 9, 23)).unwrap().unwrap();
        let times: Vec<(String, i64)> = day
            .time_per_project(vault.projects())
            .into_iter()
            .map(|(slug, time)| (slug.to_string(), time.num_minutes()))
            .collect();
        // The lunch break is left out, the deployment past midnight counts.
        assert_eq!(
            times,
            [
                ("filler".to_owned(), 30),
                ("infra".to_owned(), 135 + 120),
                ("meetings".to_owned(), 15),
                ("webshop".to_owned(), 105),
            ]
        );
    }
}
