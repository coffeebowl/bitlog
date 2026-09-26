//! Read access to a whole vault.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Months, NaiveDate};

use crate::error::{ReadError, SaveError};
use crate::file::{content_hash, read_optional, read_text, write_atomic};
use crate::watch::{OwnWrites, VaultChange, VaultWatcher, WatchError, watch};
use crate::{Day, DayWarning, EditError, Project, ProjectSlug, VaultConfig};

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
    config: VaultConfig,
    /// Sorted by slug.
    projects: Vec<Project>,
    /// Shared by all clones, so that watching knows about every write.
    own_writes: OwnWrites,
}

/// A day as read from its file, remembering the file's content to notice
/// changes made elsewhere before saving.
#[derive(Debug, Clone)]
pub struct DayFile {
    pub day: Day,
    pub warnings: Vec<DayWarning>,
    /// `None` while there is no file for the day.
    hash: Option<u64>,
}

impl DayFile {
    /// A day that has no file yet.
    fn new(day: Day) -> Self {
        Self {
            day,
            warnings: Vec::new(),
            hash: None,
        }
    }

    fn read(path: &Path, text: &str) -> Result<Self, ReadError> {
        let (day, warnings) = Day::read(path, text)?;
        Ok(Self {
            day,
            warnings,
            hash: Some(content_hash(text)),
        })
    }
}

impl Vault {
    /// Opens the vault in the folder `root`, reading its configuration and
    /// projects. Days are read on demand.
    pub fn open(root: &Path) -> Result<Self, ReadError> {
        Ok(Self {
            root: root.to_owned(),
            config: VaultConfig::load(&root.join("knotbook.toml"))?,
            projects: Project::load_all(root)?,
            own_writes: OwnWrites::default(),
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
    pub fn load_day(&self, date: NaiveDate) -> Result<Option<DayFile>, ReadError> {
        let path = self.day_path(date);
        read_optional(&path)?
            .map(|text| DayFile::read(&path, &text))
            .transpose()
    }

    /// A day without a file yet, with the vault's default location.
    pub fn new_day(&self, date: NaiveDate) -> DayFile {
        DayFile::new(Day {
            location: self.config.defaults.location.clone(),
            ..Day::new(date)
        })
    }

    /// Applies `change` to the day of `file` and saves it, creating the file
    /// if needed. Returns the day as saved.
    ///
    /// If the file was changed elsewhere since `file` was read, `change` is
    /// applied to the day as it is now instead, so that both changes are kept.
    pub fn update_day(
        &self,
        file: &DayFile,
        change: impl FnOnce(&mut Day) -> Result<(), EditError>,
    ) -> Result<DayFile, SaveError> {
        let path = self.day_path(file.day.date);
        let current = read_optional(&path)?;
        let base = if current.as_deref().map(content_hash) == file.hash {
            file.clone()
        } else if let Some(text) = &current {
            DayFile::read(&path, text)?
        } else {
            self.new_day(file.day.date)
        };
        let mut day = base.day.clone();
        change(&mut day)?;
        // Leaves files that are not in canonical form alone.
        if day == base.day && current.is_some() {
            return Ok(base);
        }
        let text = day.to_markdown();
        // Read back before writing, so that a bug never leaves an unreadable file.
        let saved = DayFile::read(&path, &text).expect("changes keep a day valid and on its date");
        self.write(&path, &text)?;
        Ok(saved)
    }

    /// Applies `change` to the project `slug` of this vault and saves it.
    ///
    /// If the file was changed elsewhere since it was read, `change` is applied
    /// to the project as it is now instead, so that both changes are kept.
    pub fn update_project(
        &mut self,
        slug: &ProjectSlug,
        change: impl FnOnce(&mut Project),
    ) -> Result<&Project, SaveError> {
        let index = self
            .projects
            .iter()
            .position(|project| project.slug == *slug)
            .expect("only projects of the vault are updated");
        let path = Project::path(&self.root, slug);
        let current = read_text(&path)?;
        let mut project = if self.projects[index].is_read_from(&current) {
            self.projects[index].clone()
        } else {
            Project::read(&self.root, slug.clone(), &current)?
        };
        change(&mut project);
        let text = project.to_toml();
        let saved =
            Project::read(&self.root, slug.clone(), &text).expect("changes keep a project valid");
        // An unchanged project comes out byte-identical.
        if current != text {
            self.write(&path, &text)?;
        }
        self.projects[index] = saved;
        Ok(&self.projects[index])
    }

    /// Watches the vault for files changed elsewhere, such as by a sync tool
    /// or the CLI, and passes them to `on_change` on a thread of its own.
    /// Writes through this vault or its clones are left out. Watching stops
    /// when the returned watcher is dropped.
    pub fn watch(
        &self,
        on_change: impl Fn(Result<Vec<VaultChange>, WatchError>) + Send + 'static,
    ) -> Result<VaultWatcher, WatchError> {
        watch(&self.root, self.own_writes.clone(), on_change)
    }

    fn write(&self, path: &Path, text: &str) -> Result<(), SaveError> {
        let relative = path
            .strip_prefix(&self.root)
            .expect("vault files lie in the vault");
        // Before writing, so that watching never sees the file first.
        self.own_writes.record(relative, text);
        write_atomic(path, text)
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

/// The date of a day file named `name`. Only names of the exact form
/// `YYYY-MM-DD.md` count, which leaves out sync conflict copies.
pub(crate) fn day_file_date(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_suffix(".md")?;
    // Parsing alone would also accept `2026-9-1`.
    NaiveDate::parse_from_str(stem, "%Y-%m-%d")
        .ok()
        .filter(|date| date.format("%Y-%m-%d").to_string() == stem)
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
        let Some(date) = entry.file_name().to_str().and_then(day_file_date) else {
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
    use std::collections::BTreeSet;

    use chrono::TimeDelta;

    use super::*;
    use crate::RemovedText;
    use crate::file::TempDir;

    fn sample_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault")
    }

    fn sample_vault() -> Vault {
        Vault::open(&sample_path()).unwrap()
    }

    /// A copy of the sample vault that tests may change.
    fn sample_copy() -> (TempDir, Vault) {
        fn copy(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let dir = TempDir::new();
        copy(&sample_path(), &dir.0);
        let vault = Vault::open(&dir.0).unwrap();
        (dir, vault)
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
        let day = vault.load_day(date(2026, 9, 22)).unwrap().unwrap().day;
        assert_eq!(day.note, "Office day, lots of reviews.");
        assert!(vault.load_day(date(2026, 9, 24)).unwrap().is_none());
    }

    #[test]
    fn working_time() {
        let vault = sample_vault();
        let hours = |day: u32| {
            let day = vault.load_day(date(2026, 9, day)).unwrap().unwrap().day;
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
        let day = vault.load_day(date(2026, 9, 23)).unwrap().unwrap().day;
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

    #[test]
    fn update_day() {
        let (_dir, vault) = sample_copy();
        let file = vault.load_day(date(2026, 9, 21)).unwrap().unwrap();
        let saved = vault
            .update_day(&file, |day| {
                day.note = "Changed.".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.day.note, "Changed.");
        let path = vault.day_path(date(2026, 9, 21));
        assert_eq!(fs::read_to_string(&path).unwrap(), saved.day.to_markdown());

        // The saved state is the base for the next change, without reading again.
        let again = vault
            .update_day(&saved, |day| {
                day.kind = "vacation".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(again.day.note, "Changed.");
        assert_eq!(
            vault.load_day(date(2026, 9, 21)).unwrap().unwrap().day,
            again.day
        );
    }

    #[test]
    fn update_day_keeps_external_changes() {
        let (_dir, vault) = sample_copy();
        let file = vault.load_day(date(2026, 9, 21)).unwrap().unwrap();
        let path = vault.day_path(date(2026, 9, 21));
        let external = fs::read_to_string(&path)
            .unwrap()
            .replace("location: \"remote\"", "location: \"office\"");
        fs::write(&path, external).unwrap();

        let saved = vault
            .update_day(&file, |day| {
                day.note = "Changed.".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.day.location, Some("office".parse().unwrap()));
        assert_eq!(saved.day.note, "Changed.");
        assert_eq!(
            vault.load_day(date(2026, 9, 21)).unwrap().unwrap().day,
            saved.day
        );
    }

    #[test]
    fn change_that_no_longer_fits_is_not_saved() {
        let (_dir, vault) = sample_copy();
        let date = date(2026, 9, 21);
        let file = vault.load_day(date).unwrap().unwrap();
        let first = file.day.blocks[0].id.clone();
        // Elsewhere, the block is removed.
        let path = vault.day_path(date);
        let mut external = file.day.clone();
        external.remove_block(&first, RemovedText::Discard).unwrap();
        fs::write(&path, external.to_markdown()).unwrap();

        let err = vault
            .update_day(&file, |day| day.set_block_title(&first, "Late"))
            .unwrap_err();
        assert!(
            matches!(err, SaveError::Edit(EditError::UnknownBlock(_))),
            "{err}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), external.to_markdown());
    }

    #[test]
    fn update_new_day() {
        let (_dir, vault) = sample_copy();
        let file = vault.new_day(date(2026, 10, 1));
        let saved = vault
            .update_day(&file, |day| {
                day.note = "First.".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.day.note, "First.");
        assert_eq!(saved.day.location, Some("remote".parse().unwrap()));
        assert_eq!(
            vault.load_day(date(2026, 10, 1)).unwrap().unwrap().day,
            saved.day
        );

        // Created elsewhere in the meantime: the change applies to that file.
        let saved = vault
            .update_day(&file, |day| {
                day.kind = "vacation".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(saved.day.note, "First.");
        assert_eq!(saved.day.kind, "vacation");
    }

    #[test]
    fn unchanged_day_is_not_written() {
        let (_dir, vault) = sample_copy();
        // Not in canonical form, so writing would change it.
        let path = vault.day_path(date(2026, 9, 22));
        let before = fs::read_to_string(&path).unwrap();
        let file = vault.load_day(date(2026, 9, 22)).unwrap().unwrap();
        vault.update_day(&file, |_| Ok(())).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn update_project() {
        let (_dir, mut vault) = sample_copy();
        let slug: ProjectSlug = "infra".parse().unwrap();
        let path = Project::path(vault.root(), &slug);
        let external = fs::read_to_string(&path)
            .unwrap()
            .replace("pinned = false", "pinned = true");
        fs::write(&path, &external).unwrap();

        let project = vault
            .update_project(&slug, |project| project.name = "Infra".to_owned())
            .unwrap();
        assert_eq!(project.name, "Infra");
        assert!(project.pinned);
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, external.replace("\"Infrastructure\"", "\"Infra\""));
        assert_eq!(vault.project(&slug).unwrap().name, "Infra");
    }

    #[test]
    fn watch_reports_only_changes_made_elsewhere() {
        let (_dir, vault) = sample_copy();
        let (sender, changes) = std::sync::mpsc::channel();
        let _watcher = vault
            .watch(move |result| sender.send(result.unwrap()).unwrap())
            .unwrap();
        let next = || {
            changes
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
        };

        let path = vault.day_path(date(2026, 9, 22));
        fs::write(&path, fs::read_to_string(&path).unwrap() + "\nMore.\n").unwrap();
        assert_eq!(next(), [VaultChange::Day(date(2026, 9, 22))]);

        // Own writes, a new month folder included, are left out; only the
        // changes made afterwards are reported.
        let file = vault.new_day(date(2026, 10, 1));
        vault
            .update_day(&file, |day| {
                day.note = "Mine.".to_owned();
                Ok(())
            })
            .unwrap();
        let project = Project::path(vault.root(), &"infra".parse().unwrap());
        fs::write(
            &project,
            fs::read_to_string(&project).unwrap() + "# Theirs.\n",
        )
        .unwrap();
        fs::remove_file(vault.root().join("knotbook.toml")).unwrap();
        let mut reported = BTreeSet::new();
        while !reported.contains(&VaultChange::Config) || reported.len() < 2 {
            reported.extend(next());
        }
        // The first change may be reported once more, split over two batches.
        reported.remove(&VaultChange::Day(date(2026, 9, 22)));
        assert_eq!(
            Vec::from_iter(reported),
            [
                VaultChange::Config,
                VaultChange::Project("infra".parse().unwrap())
            ]
        );
    }
}
