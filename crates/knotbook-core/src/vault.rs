//! Read access to a whole vault.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Months, NaiveDate};

use crate::error::{ReadError, SaveError};
use crate::file::{content_hash, read_optional, read_text, write_atomic};
use crate::watch::{OwnWrites, VaultChange, VaultWatcher, WatchError, watch};
use crate::{Day, DayWarning, EditError, Project, ProjectSlug, TaskList, VaultConfig};

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

    /// Saves the new project `project` and adds it to the vault.
    pub fn add_project(&mut self, project: Project) -> Result<&Project, SaveError> {
        let path = Project::path(&self.root, &project.slug);
        if self.project(&project.slug).is_some() || path.exists() {
            return Err(EditError::ProjectExists(project.slug).into());
        }
        let text = project.to_toml();
        let saved =
            Project::read(&self.root, project.slug.clone(), &text).expect("new projects are valid");
        self.write(&path, &text)?;
        let index = self
            .projects
            .partition_point(|other| other.slug < saved.slug);
        self.projects.insert(index, saved);
        Ok(&self.projects[index])
    }

    /// Applies `change` to the project `slug` of this vault and saves it.
    ///
    /// If the file was changed elsewhere since it was read, `change` is applied
    /// to the project as it is now instead, so that both changes are kept.
    pub fn update_project(
        &mut self,
        slug: &ProjectSlug,
        change: impl FnOnce(&mut Project) -> Result<(), EditError>,
    ) -> Result<&Project, SaveError> {
        let index = self
            .projects
            .iter()
            .position(|project| project.slug == *slug)
            .ok_or_else(|| EditError::UnknownProject(slug.clone()))?;
        let path = Project::path(&self.root, slug);
        let current = read_text(&path)?;
        let mut project = if self.projects[index].is_read_from(&current) {
            self.projects[index].clone()
        } else {
            Project::read(&self.root, slug.clone(), &current)?
        };
        change(&mut project)?;
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

    /// Where exports go. Knotbook may overwrite anything in there.
    pub fn exports_path(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// Writes `text` to the file `name` in the exports folder, replacing
    /// what is there. Returns the path written.
    pub fn write_export(&self, name: &str, text: &str) -> Result<PathBuf, SaveError> {
        let path = self.exports_path().join(name);
        self.write(&path, text)?;
        Ok(path)
    }

    /// Where the global task list lives.
    pub fn tasks_path(&self) -> PathBuf {
        self.root.join("tasks.toml")
    }

    /// Where the tasks finished in `year` are archived.
    pub fn task_archive_path(&self, year: i32) -> PathBuf {
        self.root.join(format!("tasks-archive-{year:04}.toml"))
    }

    /// Reads the global task list, which is empty while there is no file.
    pub fn load_tasks(&self) -> Result<TaskList, ReadError> {
        let path = self.tasks_path();
        match read_optional(&path)? {
            Some(text) => TaskList::read(&path, &text),
            None => Ok(TaskList::default()),
        }
    }

    /// Applies `change` to the task list `tasks` and saves it. The file is
    /// created with the first task. Returns the list as saved.
    ///
    /// If the file was changed elsewhere since `tasks` was read, `change` is
    /// applied to the list as it is now instead, so that both changes are kept.
    pub fn update_tasks(
        &self,
        tasks: &TaskList,
        change: impl FnOnce(&mut TaskList) -> Result<(), EditError>,
    ) -> Result<TaskList, SaveError> {
        let (current, mut tasks) = self.current_tasks(tasks)?;
        change(&mut tasks)?;
        self.save_tasks(current.as_deref(), &tasks)
    }

    /// Moves all done and dropped tasks of the task list `tasks` to the
    /// archive of the year they were finished in, or created in if that is
    /// not known, or else of `today`. Returns the list as saved.
    pub fn archive_tasks(&self, tasks: &TaskList, today: NaiveDate) -> Result<TaskList, SaveError> {
        let (current, mut tasks) = self.current_tasks(tasks)?;
        let mut years = BTreeMap::<i32, Vec<_>>::new();
        for task in tasks.take_finished() {
            let date = task.done.or(task.created).unwrap_or(today);
            years.entry(date.year()).or_default().push(task);
        }
        // Archives first: if writing the list fails, tasks are archived
        // twice rather than lost.
        for (year, finished) in years {
            let path = self.task_archive_path(year);
            let text = read_optional(&path)?;
            let mut archive = match &text {
                Some(text) => TaskList::read(&path, text)?,
                None => TaskList::default(),
            };
            for task in finished {
                archive.push_finished(task);
            }
            self.write(&path, &archive.to_toml())?;
        }
        self.save_tasks(current.as_deref(), &tasks)
    }

    /// The content of the task file now, and the list it holds: `tasks`
    /// if the file is still as `tasks` was read from.
    fn current_tasks(&self, tasks: &TaskList) -> Result<(Option<String>, TaskList), ReadError> {
        let path = self.tasks_path();
        let current = read_optional(&path)?;
        let tasks = if tasks.is_read_from(current.as_deref()) {
            tasks.clone()
        } else if let Some(text) = &current {
            TaskList::read(&path, text)?
        } else {
            TaskList::default()
        };
        Ok((current, tasks))
    }

    /// Writes `tasks` unless the file `current` stays the same, or no file
    /// would be created just to hold no tasks.
    fn save_tasks(&self, current: Option<&str>, tasks: &TaskList) -> Result<TaskList, SaveError> {
        let path = self.tasks_path();
        let text = tasks.to_toml();
        let saved = TaskList::read(&path, &text).expect("changes keep a task list valid");
        let unchanged = match current {
            Some(current) => current == text,
            None => tasks.tasks().is_empty(),
        };
        if !unchanged {
            self.write(&path, &text)?;
        }
        Ok(saved)
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

    pub(crate) fn write(&self, path: &Path, text: &str) -> Result<(), SaveError> {
        self.record_write(path, Some(text));
        write_atomic(path, text)
    }

    /// Notes that this program is about to write `text` to `path`, or with
    /// `None` remove it, so that watching leaves the change out. Call it
    /// before changing the file, so that watching never sees it first.
    pub(crate) fn record_write(&self, path: &Path, text: Option<&str>) {
        let relative = path
            .strip_prefix(&self.root)
            .expect("vault files lie in the vault");
        self.own_writes.record(relative, text);
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

    /// All dates with a day file, oldest first.
    pub fn all_days(&self) -> Result<Vec<NaiveDate>, ReadError> {
        let folder = self.root().join("daily");
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(ReadError::Io {
                    path: folder,
                    source,
                });
            }
        };
        let mut dates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ReadError::Io {
                path: folder.clone(),
                source,
            })?;
            let year = entry
                .file_name()
                .to_str()
                .filter(|name| name.len() == 4)
                .and_then(|name| name.parse().ok());
            let first = year.and_then(|year| NaiveDate::from_ymd_opt(year, 1, 1));
            let last = year.and_then(|year| NaiveDate::from_ymd_opt(year, 12, 31));
            if let (Some(first), Some(last)) = (first, last) {
                dates.extend(self.days(first, last)?);
            }
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
    use crate::file::sample_copy;
    use crate::{RemovedText, TaskId, TaskStatus};

    fn sample_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault")
    }

    fn sample_vault() -> Vault {
        Vault::open(&sample_path()).unwrap()
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
            .update_project(&slug, |project| {
                project.name = "Infra".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(project.name, "Infra");
        assert!(project.pinned);
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, external.replace("\"Infrastructure\"", "\"Infra\""));
        assert_eq!(vault.project(&slug).unwrap().name, "Infra");
    }

    #[test]
    fn update_tasks_keeps_external_changes() {
        let (_dir, vault) = sample_copy();
        let tasks = vault.load_tasks().unwrap();
        let path = vault.tasks_path();
        let external = fs::read_to_string(&path)
            .unwrap()
            .replace("Get back to Kim", "Get back to Kim and Alex");
        fs::write(&path, &external).unwrap();

        let saved = vault
            .update_tasks(&tasks, |tasks| {
                tasks.add("Order coffee", date(2026, 9, 24)).map(|_| ())
            })
            .unwrap();
        let titles: Vec<&str> = saved.tasks().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles[0], "Get back to Kim and Alex about the handover");
        assert_eq!(titles[2], "Order coffee");
        assert_eq!(fs::read_to_string(&path).unwrap(), saved.to_toml());

        // The saved state is the base for the next change, without reading again.
        let id = saved.tasks()[2].id.clone();
        let again = vault
            .update_tasks(&saved, |tasks| {
                tasks.set_status(&id, TaskStatus::Done, date(2026, 9, 24))
            })
            .unwrap();
        assert_eq!(again.tasks().len(), 5);
        assert_eq!(
            vault.load_tasks().unwrap().to_toml(),
            fs::read_to_string(&path).unwrap()
        );
    }

    #[test]
    fn task_file_is_created_with_the_first_task() {
        let (_dir, vault) = sample_copy();
        fs::remove_file(vault.tasks_path()).unwrap();
        let tasks = vault.load_tasks().unwrap();
        assert!(tasks.tasks().is_empty());

        vault.update_tasks(&tasks, |_| Ok(())).unwrap();
        assert!(!vault.tasks_path().exists());

        let saved = vault
            .update_tasks(&tasks, |tasks| {
                tasks.add("First", date(2026, 10, 1)).map(|_| ())
            })
            .unwrap();
        assert_eq!(saved.tasks().len(), 1);
        assert!(vault.tasks_path().exists());
    }

    #[test]
    fn unchanged_tasks_are_not_written() {
        let (_dir, vault) = sample_copy();
        let path = vault.tasks_path();
        // Not in canonical order, so writing would change it.
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("format = 1\n", "format = 1\n\n\n");
        fs::write(&path, &text).unwrap();
        let tasks = vault.load_tasks().unwrap();
        vault.update_tasks(&tasks, |_| Ok(())).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn archive_tasks() {
        let (_dir, vault) = sample_copy();
        let archive = vault.task_archive_path(2026);
        fs::write(
            &archive,
            "format = 1\n\n[[task]]\nid = \"r3m7\"\ntitle = \"Older\"\nstatus = \"done\"\n",
        )
        .unwrap();
        let tasks = vault.load_tasks().unwrap();
        let id: TaskId = "t9x2".parse().unwrap();
        let tasks = vault
            .update_tasks(&tasks, |tasks| {
                tasks.set_status(&id, TaskStatus::Dropped, date(2027, 1, 4))
            })
            .unwrap();

        let saved = vault.archive_tasks(&tasks, date(2027, 1, 5)).unwrap();
        let open: Vec<&str> = saved.tasks().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(open, ["h4c8"]);
        assert_eq!(
            fs::read_to_string(vault.tasks_path()).unwrap(),
            saved.to_toml()
        );

        // Existing archives are extended, taken ids replaced; a dropped task
        // without a date goes by the day it was created.
        let archived = TaskList::read(&archive, &fs::read_to_string(&archive).unwrap()).unwrap();
        let titles: Vec<&str> = archived.tasks().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Older",
                "Take the keyboard to the office",
                "Book a meeting room for the retro"
            ]
        );
        assert_ne!(archived.tasks()[1].id.as_str(), "r3m7");
        let next = TaskList::read(
            &vault.task_archive_path(2027),
            &fs::read_to_string(vault.task_archive_path(2027)).unwrap(),
        )
        .unwrap();
        assert_eq!(next.tasks().len(), 1);
        assert_eq!(next.tasks()[0].id, id);

        // Nothing left to archive: nothing is written.
        let before = fs::read_to_string(vault.tasks_path()).unwrap();
        vault.archive_tasks(&saved, date(2027, 1, 5)).unwrap();
        assert_eq!(fs::read_to_string(vault.tasks_path()).unwrap(), before);
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
        let infra = "infra".parse().unwrap();
        let note = vault
            .create_note(&infra, "Mine", date(2026, 10, 1))
            .unwrap();
        vault.rename_note(&note, "Still mine", false).unwrap();
        vault
            .delete_note(&"projects/infra/notes/deployment.md".parse().unwrap())
            .unwrap();
        let project = Project::path(vault.root(), &infra);
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

    #[test]
    fn add_project() {
        let (_dir, mut vault) = sample_copy();
        let slug: ProjectSlug = "docs".parse().unwrap();
        let project = Project::new(slug.clone(), "Documentation", date(2026, 10, 1));
        let text = project.to_toml();
        vault.add_project(project).unwrap();
        assert_eq!(
            fs::read_to_string(Project::path(vault.root(), &slug)).unwrap(),
            text
        );
        let slugs: Vec<&str> = vault.projects().iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(
            slugs,
            ["docs", "filler", "infra", "meetings", "pause", "webshop"]
        );

        let again = Project::new(slug.clone(), "Again", date(2026, 10, 1));
        assert!(matches!(
            vault.add_project(again),
            Err(SaveError::Edit(EditError::ProjectExists(_)))
        ));
        // The saved project can be changed like any other.
        vault
            .update_project(&slug, |project| project.set_color("#ff7800"))
            .unwrap();
    }

    #[test]
    fn invalid_project_changes() {
        let (_dir, mut vault) = sample_copy();
        let infra: ProjectSlug = "infra".parse().unwrap();
        let before = fs::read_to_string(Project::path(vault.root(), &infra)).unwrap();
        assert!(matches!(
            vault.update_project(&infra, |project| project.set_color("green")),
            Err(SaveError::Edit(EditError::InvalidColor(_)))
        ));
        assert_eq!(
            fs::read_to_string(Project::path(vault.root(), &infra)).unwrap(),
            before
        );
        assert!(matches!(
            vault.update_project(&"nope".parse().unwrap(), |_| Ok(())),
            Err(SaveError::Edit(EditError::UnknownProject(_)))
        ));
    }
}
