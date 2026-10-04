//! Read access to a whole vault.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Months, NaiveDate};

use crate::error::{ReadError, SaveError};
use crate::file::{content_hash, read_folder, read_optional, read_text, write_atomic};
use crate::notes::relink_day;
use crate::project::project_folder;
use crate::watch::{OwnWrites, VaultChange, VaultWatcher, WatchError, watch};
use crate::{Day, DayWarning, EditError, NotePath, Project, ProjectSlug, TaskList, VaultConfig};

/// The folder exports go to, relative to the vault.
pub(crate) const EXPORTS: &str = "exports";

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
    config: VaultConfig,
    /// In the order of the settings, see [`Vault::projects`].
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

    pub(crate) fn read(path: &Path, text: &str) -> Result<Self, ReadError> {
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
        let mut vault = Self {
            root: root.to_owned(),
            config: VaultConfig::load(&Self::config_path(root))?,
            projects: Project::load_all(root)?,
            own_writes: OwnWrites::default(),
        };
        vault.sort_projects();
        Ok(vault)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &VaultConfig {
        &self.config
    }

    fn config_path(root: &Path) -> PathBuf {
        root.join("bitlog.toml")
    }

    /// Applies `change` to the settings of this vault and saves them.
    ///
    /// If the file was changed elsewhere since it was read, `change` is applied
    /// to the settings as they are now instead, so that both changes are kept.
    pub fn update_config(
        &mut self,
        change: impl FnOnce(&mut VaultConfig) -> Result<(), EditError>,
    ) -> Result<&VaultConfig, SaveError> {
        let path = Self::config_path(&self.root);
        let current = read_text(&path)?;
        let mut config = if self.config.is_read_from(&current) {
            self.config.clone()
        } else {
            VaultConfig::read(&path, &current)?
        };
        change(&mut config)?;
        let text = config.to_toml().map_err(EditError::InvalidSettings)?;
        let saved = VaultConfig::read(&path, &text).expect("valid settings are read back");
        // Unchanged settings come out byte-identical.
        if current != text {
            self.write(&path, &text)?;
        }
        self.config = saved;
        self.sort_projects();
        Ok(&self.config)
    }

    /// The projects in the order the user gave them, see
    /// [`ProjectsConfig::order`](crate::ProjectsConfig::order).
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// Sorts the projects in the order of the settings, those missing there
    /// after the others by name.
    fn sort_projects(&mut self) {
        let order = &self.config.projects.order;
        self.projects.sort_by_cached_key(|project| {
            let position = order.iter().position(|slug| *slug == project.slug);
            (
                position.unwrap_or(usize::MAX),
                project.name.to_lowercase(),
                project.slug.clone(),
            )
        });
    }

    /// Moves the project `slug` right before the project `target`, or right
    /// after it if `after`, and saves the order of all projects.
    pub fn move_project(
        &mut self,
        slug: &ProjectSlug,
        target: &ProjectSlug,
        after: bool,
    ) -> Result<(), SaveError> {
        for project in [slug, target] {
            if self.project(project).is_none() {
                return Err(EditError::UnknownProject(project.clone()).into());
            }
        }
        if slug == target {
            return Ok(());
        }
        let mut order: Vec<ProjectSlug> = self
            .projects
            .iter()
            .map(|project| project.slug.clone())
            .filter(|other| other != slug)
            .collect();
        let index = order
            .iter()
            .position(|other| other == target)
            .expect("the target is not the project moved")
            + usize::from(after);
        order.insert(index, slug.clone());
        self.update_config(|config| {
            config.projects.order = order;
            Ok(())
        })?;
        Ok(())
    }

    pub fn project(&self, slug: &ProjectSlug) -> Option<&Project> {
        self.projects.iter().find(|project| project.slug == *slug)
    }

    /// The name of the project `slug`, or the slug itself if the vault
    /// lacks it, as blocks of removed projects name it.
    pub fn project_name<'a>(&'a self, slug: &'a ProjectSlug) -> &'a str {
        self.project(slug)
            .map_or(slug.as_str(), |project| &project.name)
    }

    /// Whether blocks of the project `slug` are breaks. Blocks of projects
    /// the vault lacks are work.
    pub fn is_break(&self, slug: &ProjectSlug) -> bool {
        Project::is_break_in(&self.projects, slug)
    }

    /// Where the file of the day `date` lives.
    pub fn day_path(&self, date: NaiveDate) -> PathBuf {
        self.root.join(day_file(date))
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

    /// Deletes the file of the day `date` for good, if there is one.
    pub fn delete_day(&self, date: NaiveDate) -> Result<(), SaveError> {
        self.remove(&self.day_path(date))?;
        Ok(())
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
        self.projects.push(saved);
        self.sort_projects();
        Ok(self
            .project(&project.slug)
            .expect("the project was just added"))
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
        // The name may place it elsewhere.
        self.sort_projects();
        Ok(self.project(slug).expect("the project was just saved"))
    }

    /// Changes the slug of the project `from` to `to`: points the blocks of
    /// all days, wiki links in days and notes, the order of the projects
    /// and the repository of this device to `to`, then moves the project's
    /// folder. Returns the number of days and notes changed.
    ///
    /// Nothing is changed while a day cannot be read. The folder is moved
    /// last, so that renaming again after an interruption finishes the job.
    pub fn rename_project(
        &mut self,
        from: &ProjectSlug,
        to: &ProjectSlug,
    ) -> Result<(usize, usize), SaveError> {
        if self.project(from).is_none() {
            return Err(EditError::UnknownProject(from.clone()).into());
        }
        let (old, new) = (
            project_folder(&self.root, from),
            project_folder(&self.root, to),
        );
        if self.project(to).is_some() || new.exists() {
            return Err(EditError::ProjectExists(to.clone()).into());
        }
        for date in self.all_days()? {
            self.load_day(date)?;
        }
        let target = |note: &NotePath| {
            (note.project() == from)
                .then(|| NotePath::new(to.clone(), note.name()).expect("the name stays valid"))
        };
        let days = self.change_days(|day| {
            let mut changed = relink_day(day, &target);
            for block in day.blocks.iter_mut().filter(|block| block.project == *from) {
                block.project = to.clone();
                changed = true;
            }
            changed
        })?;
        let notes = self.relink_notes(&target)?;
        if self.config.projects.order.contains(from) {
            self.update_config(|config| {
                for slug in &mut config.projects.order {
                    if slug == from {
                        *slug = to.clone();
                    }
                }
                Ok(())
            })?;
        }
        self.rename_repo_path(from, to)?;
        fs::rename(&old, &new).map_err(|source| SaveError::Write {
            path: new.clone(),
            source,
        })?;
        let path = Project::path(&self.root, to);
        let project = Project::read(&self.root, to.clone(), &read_text(&path)?)?;
        self.projects.retain(|project| project.slug != *from);
        self.projects.push(project);
        self.sort_projects();
        Ok((days, notes))
    }

    /// Where exports go. BitLog may overwrite anything in there.
    fn exports_path(&self) -> PathBuf {
        self.root.join(EXPORTS)
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
        TaskList::load(&self.tasks_path())
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
            let mut archive = TaskList::load(&path)?;
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
    /// A conflict copy is passed as a change of its original. Writes
    /// through this vault or its clones are left out. Watching stops
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

    /// Removes the file at `path`. Returns whether there was one.
    pub(crate) fn remove(&self, path: &Path) -> Result<bool, SaveError> {
        self.record_write(path, None);
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(SaveError::Write {
                path: path.to_owned(),
                source,
            }),
        }
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
        let mut dates = Vec::new();
        for entry in read_folder(&self.root().join("daily"))? {
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

/// Where the file of the day `date` lies, relative to the vault.
pub(crate) fn day_file(date: NaiveDate) -> PathBuf {
    Path::new("daily")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{date}.md"))
}

/// The date of a day file named `name`. Only names of the exact form
/// `YYYY-MM-DD.md` count, which leaves out sync conflict copies.
pub(crate) fn day_file_date(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_suffix(".md")?;
    // Parsing alone would also accept `2026-9-1`.
    NaiveDate::parse_from_str(stem, "%Y-%m-%d")
        .ok()
        .filter(|date| date.to_string() == stem)
}

/// The dates of the day files in the month folder `folder`.
///
/// Only names of the exact form `YYYY-MM-DD.md` of that month count, which
/// leaves out sync conflict copies and any other file.
fn day_files(folder: &Path) -> Result<Vec<NaiveDate>, ReadError> {
    let mut dates = Vec::new();
    for entry in read_folder(folder)? {
        let Some(date) = entry.file_name().to_str().and_then(day_file_date) else {
            continue;
        };
        if folder.ends_with(day_file(date).parent().expect("day files lie in a folder")) {
            dates.push(date);
        }
    }
    Ok(dates)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

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
        assert_eq!(vault.config().name, "Sample BitLog");
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
        // Blocks from 08:00 to 16:30 without gaps, minus a 45 minute break.
        assert_eq!(hours(21), TimeDelta::minutes(7 * 60 + 45));
        // From 08:45 to 17:15, minus a 45 minute break.
        assert_eq!(hours(22), TimeDelta::minutes(7 * 60 + 45));
        // All blocks but the break, with gaps and one past midnight.
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
    fn delete_day() {
        let (_dir, vault) = sample_copy();
        vault.delete_day(date(2026, 9, 21)).unwrap();
        assert!(vault.load_day(date(2026, 9, 21)).unwrap().is_none());
        assert!(vault.load_day(date(2026, 9, 22)).unwrap().is_some());
        vault.delete_day(date(2026, 9, 21)).unwrap();
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
            .replace("category = \"work\"", "category = \"ops\"");
        fs::write(&path, &external).unwrap();

        let project = vault
            .update_project(&slug, |project| {
                project.name = "Infra".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(project.name, "Infra");
        assert_eq!(project.category, "ops");
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, external.replace("\"Infrastructure\"", "\"Infra\""));
        assert_eq!(vault.project(&slug).unwrap().name, "Infra");
    }

    fn slugs(vault: &Vault) -> Vec<&str> {
        vault.projects().iter().map(|p| p.slug.as_str()).collect()
    }

    #[test]
    fn projects_come_in_the_order_of_the_settings() {
        let (_dir, vault) = sample_copy();
        // Break is missing from the order.
        assert_eq!(
            slugs(&vault),
            ["webshop", "infra", "meetings", "filler", "pause"]
        );
    }

    #[test]
    fn move_project() {
        let (_dir, mut vault) = sample_copy();
        let slug = |value: &str| -> ProjectSlug { value.parse().unwrap() };
        vault
            .move_project(&slug("pause"), &slug("infra"), false)
            .unwrap();
        assert_eq!(
            slugs(&vault),
            ["webshop", "pause", "infra", "meetings", "filler"]
        );
        vault
            .move_project(&slug("webshop"), &slug("filler"), true)
            .unwrap();
        let expected = ["pause", "infra", "meetings", "filler", "webshop"];
        assert_eq!(slugs(&vault), expected);
        // Saved for the next start.
        assert_eq!(slugs(&Vault::open(vault.root()).unwrap()), expected);
        let text = fs::read_to_string(vault.root().join("bitlog.toml")).unwrap();
        assert!(
            text.contains("order = [\"pause\", \"infra\", \"meetings\", \"filler\", \"webshop\"]"),
            "{text}"
        );
        assert!(
            vault
                .move_project(&slug("nope"), &slug("infra"), false)
                .is_err()
        );
    }

    #[test]
    fn update_config() {
        let (_dir, mut vault) = sample_copy();
        let path = vault.root().join("bitlog.toml");
        let external = fs::read_to_string(&path)
            .unwrap()
            .replace("target_hours = 32.0", "target_hours = 30.0");
        fs::write(&path, &external).unwrap();

        let config = vault
            .update_config(|config| {
                config.grid.slot_minutes = 30;
                Ok(())
            })
            .unwrap();
        assert_eq!(config.grid.slot_minutes, 30);
        assert_eq!(config.week.target_hours, 30.0);
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            external.replace("slot_minutes = 15", "slot_minutes = 30")
        );

        let err = vault
            .update_config(|config| {
                config.grid.slot_minutes = 7;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(
            err,
            SaveError::Edit(EditError::InvalidSettings(_))
        ));
        assert_eq!(vault.config().grid.slot_minutes, 30);
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
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
        fs::remove_file(vault.root().join("bitlog.toml")).unwrap();
        // A conflict copy counts as a change of its original.
        fs::write(vault.root().join("tasks (conflicted copy).toml"), "").unwrap();
        let mut reported = BTreeSet::new();
        while !reported.contains(&VaultChange::Config) || reported.len() < 3 {
            reported.extend(next());
            // The first change may be reported once more, split over two
            // batches; counting it would end the loop too early.
            reported.remove(&VaultChange::Day(date(2026, 9, 22)));
        }
        assert_eq!(
            Vec::from_iter(reported),
            [
                VaultChange::Config,
                VaultChange::Tasks,
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
        // After the ordered projects, by name: Break, then Documentation.
        assert_eq!(
            slugs(&vault),
            ["webshop", "infra", "meetings", "filler", "pause", "docs"]
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
    fn rename_project() {
        let (_dir, mut vault) = sample_copy();
        let infra: ProjectSlug = "infra".parse().unwrap();
        let platform: ProjectSlug = "platform".parse().unwrap();
        let repo = vault.root().join("code");
        fs::create_dir_all(repo.join(".git")).unwrap();
        vault.set_repo_path(&infra, Some(&repo)).unwrap();

        // Days 21 to 23 have infra blocks, only checkout-flow links to an
        // infra note.
        assert_eq!(vault.rename_project(&infra, &platform).unwrap(), (3, 1));
        assert_eq!(
            slugs(&vault),
            ["webshop", "platform", "meetings", "filler", "pause"]
        );
        assert!(!vault.root().join("projects/infra").exists());
        let deployment: NotePath = "projects/platform/notes/deployment.md".parse().unwrap();
        assert!(vault.note_path(&deployment).is_file());
        let checkout = "projects/webshop/notes/checkout-flow.md".parse().unwrap();
        assert!(
            vault
                .load_note(&checkout)
                .unwrap()
                .text
                .contains("[[platform/deployment]]")
        );
        let day = vault.load_day(date(2026, 9, 23)).unwrap().unwrap().day;
        assert!(day.blocks.iter().any(|block| block.project == platform));
        assert!(
            day.blocks
                .iter()
                .any(|block| block.text.contains("[[platform/deployment]]"))
        );
        for date in vault.all_days().unwrap() {
            let day = vault.load_day(date).unwrap().unwrap().day;
            assert!(day.blocks.iter().all(|block| block.project != infra));
        }
        assert_eq!(
            vault.repo_paths().unwrap(),
            BTreeMap::from([(platform.clone(), repo)])
        );
        let reopened = Vault::open(vault.root()).unwrap();
        assert_eq!(slugs(&reopened), slugs(&vault));

        let meetings: ProjectSlug = "meetings".parse().unwrap();
        assert!(matches!(
            vault.rename_project(&infra, &meetings),
            Err(SaveError::Edit(EditError::UnknownProject(_)))
        ));
        assert!(matches!(
            vault.rename_project(&platform, &meetings),
            Err(SaveError::Edit(EditError::ProjectExists(_)))
        ));
    }

    #[test]
    fn rename_project_needs_readable_days() {
        let (_dir, mut vault) = sample_copy();
        let path = vault.day_path(date(2026, 9, 22));
        fs::write(&path, "---\nnot: [valid\n").unwrap();
        let infra: ProjectSlug = "infra".parse().unwrap();
        let platform: ProjectSlug = "platform".parse().unwrap();
        assert!(matches!(
            vault.rename_project(&infra, &platform),
            Err(SaveError::Read(_))
        ));
        assert!(vault.project(&infra).is_some());
        let day = vault.load_day(date(2026, 9, 21)).unwrap().unwrap().day;
        assert!(day.blocks.iter().any(|block| block.project == infra));
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
