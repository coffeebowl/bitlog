//! The global task list, `tasks.toml`, and its yearly archives.

use std::collections::HashSet;
use std::path::Path;

use chrono::NaiveDate;
use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::conflict::Merger;
use crate::error::ReadError;
use crate::file::{content_hash, parse_text};
use crate::toml_values::{local_date, same_item, set, set_date};
use crate::{Contradiction, EditError, TaskId};

/// The only format version this code knows.
const FORMAT: u32 = 1;

/// Task fields of format version 1. Everything else is kept as is.
const KNOWN_FIELDS: [&str; 6] = ["id", "title", "status", "created", "due", "done"];

#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub created: Option<NaiveDate>,
    pub due: Option<NaiveDate>,
    /// When the task was done or dropped.
    pub done: Option<NaiveDate>,
    /// The table as read, so that writing keeps comments, formatting and
    /// fields this version does not know.
    table: Table,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Open,
    Done,
    Dropped,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }
}

/// The tasks of `tasks.toml` or of an archive, open tasks first.
#[derive(Debug, Clone, Default)]
pub struct TaskList {
    tasks: Vec<Task>,
    /// The file as read, so that writing keeps comments and formatting.
    document: DocumentMut,
    /// Of the file as read, `None` if there was no file.
    hash: Option<u64>,
}

#[derive(Deserialize)]
struct TasksFile {
    format: u32,
    #[serde(default)]
    task: Vec<TaskEntry>,
}

#[derive(Deserialize)]
struct TaskEntry {
    id: TaskId,
    title: String,
    #[serde(default)]
    status: TaskStatus,
    #[serde(default, deserialize_with = "local_date")]
    created: Option<NaiveDate>,
    #[serde(default, deserialize_with = "local_date")]
    due: Option<NaiveDate>,
    #[serde(default, deserialize_with = "local_date")]
    done: Option<NaiveDate>,
}

impl Task {
    pub fn is_open(&self) -> bool {
        self.status == TaskStatus::Open
    }

    fn to_table(&self) -> Table {
        let mut table = self.table.clone();
        set(&mut table, "id", self.id.as_str().into());
        set(&mut table, "title", self.title.as_str().into());
        set(&mut table, "status", self.status.as_str().into());
        set_date(&mut table, "created", self.created);
        set_date(&mut table, "due", self.due);
        set_date(&mut table, "done", self.done);
        table
    }
}

/// A title as it is kept: trimmed, not empty and on one line.
fn task_title(title: &str) -> Result<String, EditError> {
    let title = title.trim();
    if title.is_empty() || title.contains(['\n', '\r']) {
        return Err(EditError::InvalidTaskTitle);
    }
    Ok(title.to_owned())
}

impl TaskList {
    /// All tasks, open tasks first, each part in the order chosen by the user.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == *id)
    }

    /// Adds an open task after the other open tasks and returns its id.
    pub fn add(&mut self, title: &str, today: NaiveDate) -> Result<TaskId, EditError> {
        let title = task_title(title)?;
        let id = TaskId::generate(|id| self.task(id).is_some());
        let task = Task {
            id: id.clone(),
            title,
            status: TaskStatus::Open,
            created: Some(today),
            due: None,
            done: None,
            table: Table::new(),
        };
        self.tasks.insert(self.open_count(), task);
        Ok(id)
    }

    pub fn set_title(&mut self, id: &TaskId, title: &str) -> Result<(), EditError> {
        let title = task_title(title)?;
        self.task_mut(id)?.title = title;
        Ok(())
    }

    pub fn set_due(&mut self, id: &TaskId, due: Option<NaiveDate>) -> Result<(), EditError> {
        self.task_mut(id)?.due = due;
        Ok(())
    }

    /// Opens, finishes or drops a task. An open task that is finished or
    /// dropped gets `today` as its `done` date and moves to the top of the
    /// finished tasks; a task opened again moves to the end of the open ones.
    /// Between done and dropped, a task keeps its place and date.
    pub fn set_status(
        &mut self,
        id: &TaskId,
        status: TaskStatus,
        today: NaiveDate,
    ) -> Result<(), EditError> {
        let index = self.index(id)?;
        let task = &mut self.tasks[index];
        let was_open = task.is_open();
        task.status = status;
        task.done = match status {
            TaskStatus::Open => None,
            _ if was_open => Some(today),
            _ => task.done.or(Some(today)),
        };
        if was_open != task.is_open() {
            let task = self.tasks.remove(index);
            self.tasks.insert(self.open_count(), task);
        }
        Ok(())
    }

    /// Moves a task to `index` in `tasks()`. Open tasks always stay before
    /// finished ones, so the index is kept within the task's part.
    pub fn move_task(&mut self, id: &TaskId, index: usize) -> Result<(), EditError> {
        let task = self.tasks.remove(self.index(id)?);
        let open = self.open_count();
        let index = if task.is_open() {
            index.min(open)
        } else {
            index.clamp(open, self.tasks.len())
        };
        self.tasks.insert(index, task);
        Ok(())
    }

    /// Removes the finished tasks and returns them, for archiving.
    pub(crate) fn take_finished(&mut self) -> Vec<Task> {
        let open = self.open_count();
        self.tasks.split_off(open)
    }

    /// Adds a finished task at the end, with a new id if its id is taken.
    pub(crate) fn push_finished(&mut self, mut task: Task) {
        if self.task(&task.id).is_some() {
            task.id = TaskId::generate(|id| self.task(id).is_some());
        }
        self.tasks.push(task);
    }

    fn open_count(&self) -> usize {
        self.tasks.partition_point(Task::is_open)
    }

    fn index(&self, id: &TaskId) -> Result<usize, EditError> {
        self.tasks
            .iter()
            .position(|task| task.id == *id)
            .ok_or_else(|| EditError::UnknownTask(id.clone()))
    }

    fn task_mut(&mut self, id: &TaskId) -> Result<&mut Task, EditError> {
        let index = self.index(id)?;
        Ok(&mut self.tasks[index])
    }

    /// Takes from `other`, a conflict copy of this list, what does not
    /// contradict it: tasks by their ids, and each field that is the same
    /// on both sides or missing on one. Tasks with an id in `archived` that
    /// only one side has were archived on the other and are left out. What
    /// contradicts is noted in `merger` and taken from `other` only if
    /// decided so.
    pub(crate) fn merge(
        &mut self,
        other: &TaskList,
        archived: &HashSet<TaskId>,
        merger: &mut Merger,
    ) {
        let theirs_ids: HashSet<&TaskId> = other.tasks.iter().map(|task| &task.id).collect();
        self.tasks
            .retain(|task| theirs_ids.contains(&task.id) || !archived.contains(&task.id));
        for theirs in &other.tasks {
            let Some(ours) = self.tasks.iter_mut().find(|task| task.id == theirs.id) else {
                if !archived.contains(&theirs.id) {
                    self.tasks.push(theirs.clone());
                }
                continue;
            };
            let id = ours.id.clone();
            let field = |name: &str| {
                let (id, name) = (id.clone(), name.to_owned());
                move || Contradiction::TaskField(id, name)
            };
            let (title, status) = (field("title"), field("status"));
            let (created, due, done) = (field("created"), field("due"), field("done"));
            merger.value(&mut ours.title, &theirs.title, &String::new(), title);
            if ours.status != theirs.status && merger.contradiction(status()) {
                ours.status = theirs.status;
            }
            merger.value(&mut ours.created, &theirs.created, &None, created);
            merger.value(&mut ours.due, &theirs.due, &None, due);
            merger.value(&mut ours.done, &theirs.done, &None, done);
            for (key, item) in theirs.table.iter() {
                if KNOWN_FIELDS.contains(&key) {
                    continue;
                }
                let agreed = ours
                    .table
                    .get(key)
                    .is_none_or(|value| same_item(value, item));
                if ours.table.get(key).is_none() || (!agreed && merger.contradiction(field(key)()))
                {
                    ours.table.insert(key, item.clone());
                }
            }
        }
        // Stable, so that tasks added from the copy follow the others.
        self.tasks.sort_by_key(|task| !task.is_open());
    }

    /// Reads the content `text` of the task file at `path`.
    pub(crate) fn read(path: &Path, text: &str) -> Result<Self, ReadError> {
        parse_text(path, text, Self::parse)
    }

    /// Whether this list was read from a file with the content `text`.
    pub(crate) fn is_read_from(&self, text: Option<&str>) -> bool {
        text.map(content_hash) == self.hash
    }

    fn parse(text: &str) -> Result<Self, String> {
        let file: TasksFile = toml::from_str(text).map_err(|err| err.to_string())?;
        // Read a second time, keeping comments and formatting for writing.
        let document: DocumentMut = text
            .parse()
            .map_err(|err: toml_edit::TomlError| err.to_string())?;

        if file.format != FORMAT {
            return Err(format!("unsupported format version {}", file.format));
        }
        let mut ids = HashSet::new();
        if let Some(task) = file.task.iter().find(|task| !ids.insert(&task.id)) {
            return Err(format!("task id {} is used twice", task.id));
        }
        // Tasks may also be written as an array of inline tables.
        let tables: Vec<Table> = match document.get("task") {
            Some(item) => item
                .clone()
                .into_array_of_tables()
                .expect("a list of tasks is a list of tables")
                .into_iter()
                .collect(),
            None => Vec::new(),
        };

        let mut tasks: Vec<Task> = file
            .task
            .into_iter()
            .zip(tables)
            .map(|(entry, table)| Task {
                id: entry.id,
                title: entry.title,
                status: entry.status,
                created: entry.created,
                due: entry.due,
                done: entry.done,
                table,
            })
            .collect();
        // Stable, so the order chosen by the user is kept within each part.
        tasks.sort_by_key(|task| !task.is_open());
        Ok(Self {
            tasks,
            document,
            hash: Some(content_hash(text)),
        })
    }

    /// The content of the task file. Comments, formatting and unknown fields
    /// of the file this list was read from are kept, unchanged values are
    /// written exactly as they were.
    pub fn to_toml(&self) -> String {
        let mut document = self.document.clone();
        set(document.as_table_mut(), "format", i64::from(FORMAT).into());
        if self.tasks.is_empty() {
            document.remove("task");
            return document.to_string();
        }
        // Tables are written in the order of their positions in the file as
        // read; the same position for all keeps the order of `tasks`.
        let position = self
            .tasks
            .iter()
            .filter_map(|task| task.table.position())
            .min();
        let mut tables = ArrayOfTables::new();
        for task in &self.tasks {
            let mut table = task.to_table();
            table.set_position(position);
            tables.push(table);
        }
        document.insert("task", Item::ArrayOfTables(tables));
        document.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn sample_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault/tasks.toml")
    }

    fn sample() -> TaskList {
        TaskList::parse(&fs::read_to_string(sample_path()).unwrap()).unwrap()
    }

    fn id(value: &str) -> TaskId {
        value.parse().unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn ids(list: &TaskList) -> Vec<&str> {
        list.tasks().iter().map(|task| task.id.as_str()).collect()
    }

    #[test]
    fn sample_tasks() {
        let list = sample();
        assert_eq!(ids(&list), ["t9x2", "h4c8", "r3m7", "w5d1"]);
        let renew = list.task(&id("h4c8")).unwrap();
        assert_eq!(renew.title, "Renew the TLS certificate for staging");
        assert_eq!(renew.status, TaskStatus::Open);
        assert_eq!(renew.created, Some(date(2026, 9, 23)));
        assert_eq!(renew.due, Some(date(2026, 9, 30)));
        let keyboard = list.task(&id("r3m7")).unwrap();
        assert_eq!(keyboard.status, TaskStatus::Done);
        assert_eq!(keyboard.done, Some(date(2026, 9, 22)));
        assert_eq!(list.task(&id("w5d1")).unwrap().done, None);
    }

    #[test]
    fn unchanged_file_is_written_byte_identical() {
        let text = fs::read_to_string(sample_path()).unwrap();
        assert_eq!(TaskList::parse(&text).unwrap().to_toml(), text);
    }

    #[test]
    fn open_tasks_come_first() {
        let list = TaskList::parse(
            "format = 1\n\n[[task]]\nid = \"aaaa\"\ntitle = \"A\"\nstatus = \"done\"\n\n\
             [[task]]\nid = \"bbbb\"\ntitle = \"B\"\n\n\
             [[task]]\nid = \"cccc\"\ntitle = \"C\"\nstatus = \"dropped\"\n\n\
             [[task]]\nid = \"dddd\"\ntitle = \"D\"\nstatus = \"open\"\n",
        )
        .unwrap();
        assert_eq!(ids(&list), ["bbbb", "dddd", "aaaa", "cccc"]);
        let written = TaskList::parse(&list.to_toml()).unwrap();
        assert_eq!(ids(&written), ["bbbb", "dddd", "aaaa", "cccc"]);
    }

    #[test]
    fn add_to_missing_file() {
        let mut list = TaskList::default();
        let first = list.add("  Call Kim ", date(2026, 10, 1)).unwrap();
        let second = list.add("Water the plants", date(2026, 10, 2)).unwrap();
        assert_ne!(first, second);
        let text = list.to_toml();
        assert_eq!(
            text,
            format!(
                "format = 1\n\n\
                 [[task]]\nid = \"{first}\"\ntitle = \"Call Kim\"\nstatus = \"open\"\ncreated = 2026-10-01\n\n\
                 [[task]]\nid = \"{second}\"\ntitle = \"Water the plants\"\nstatus = \"open\"\ncreated = 2026-10-02\n"
            )
        );
        assert_eq!(TaskList::parse(&text).unwrap().to_toml(), text);
    }

    #[test]
    fn new_tasks_go_after_open_ones() {
        let mut list = sample();
        let new = list.add("Order coffee", date(2026, 9, 24)).unwrap();
        assert_eq!(ids(&list), ["t9x2", "h4c8", new.as_str(), "r3m7", "w5d1"]);
    }

    #[test]
    fn titles() {
        let mut list = sample();
        for invalid in ["", "  ", "Two\nlines"] {
            assert_eq!(
                list.add(invalid, date(2026, 9, 24)),
                Err(EditError::InvalidTaskTitle)
            );
            assert_eq!(
                list.set_title(&id("t9x2"), invalid),
                Err(EditError::InvalidTaskTitle)
            );
        }
        list.set_title(&id("t9x2"), "Get back to Kim").unwrap();
        assert_eq!(list.task(&id("t9x2")).unwrap().title, "Get back to Kim");
        assert_eq!(
            list.set_title(&id("zzzz"), "Nothing"),
            Err(EditError::UnknownTask(id("zzzz")))
        );
    }

    #[test]
    fn status_changes() {
        let mut list = sample();
        let today = date(2026, 9, 24);

        list.set_status(&id("h4c8"), TaskStatus::Done, today)
            .unwrap();
        assert_eq!(ids(&list), ["t9x2", "h4c8", "r3m7", "w5d1"]);
        assert_eq!(list.task(&id("h4c8")).unwrap().done, Some(today));

        list.set_status(&id("t9x2"), TaskStatus::Dropped, today)
            .unwrap();
        assert_eq!(ids(&list), ["t9x2", "h4c8", "r3m7", "w5d1"]);
        assert_eq!(list.open_count(), 0);

        // Finished tasks keep their date when they change between done and dropped.
        list.set_status(&id("r3m7"), TaskStatus::Dropped, today)
            .unwrap();
        assert_eq!(
            list.task(&id("r3m7")).unwrap().done,
            Some(date(2026, 9, 22))
        );
        list.set_status(&id("w5d1"), TaskStatus::Done, today)
            .unwrap();
        assert_eq!(list.task(&id("w5d1")).unwrap().done, Some(today));

        list.set_status(&id("r3m7"), TaskStatus::Open, today)
            .unwrap();
        assert_eq!(ids(&list), ["r3m7", "t9x2", "h4c8", "w5d1"]);
        let reopened = list.task(&id("r3m7")).unwrap();
        assert_eq!(reopened.done, None);
        assert!(!list.to_toml().contains("done = 2026-09-22"));
    }

    #[test]
    fn move_tasks_within_their_part() {
        let mut list = sample();
        list.move_task(&id("h4c8"), 0).unwrap();
        assert_eq!(ids(&list), ["h4c8", "t9x2", "r3m7", "w5d1"]);
        // Open tasks cannot move below finished ones and the other way round.
        list.move_task(&id("h4c8"), 3).unwrap();
        assert_eq!(ids(&list), ["t9x2", "h4c8", "r3m7", "w5d1"]);
        list.move_task(&id("w5d1"), 0).unwrap();
        assert_eq!(ids(&list), ["t9x2", "h4c8", "w5d1", "r3m7"]);

        let written = TaskList::parse(&list.to_toml()).unwrap();
        assert_eq!(ids(&written), ["t9x2", "h4c8", "w5d1", "r3m7"]);
    }

    #[test]
    fn due_dates() {
        let mut list = sample();
        list.set_due(&id("t9x2"), Some(date(2026, 10, 2))).unwrap();
        list.set_due(&id("h4c8"), None).unwrap();
        let text = list.to_toml();
        assert!(
            text.contains("created = 2026-09-22\ndue = 2026-10-02\n"),
            "{text}"
        );
        let written = TaskList::parse(&text).unwrap();
        assert_eq!(written.task(&id("h4c8")).unwrap().due, None);
    }

    #[test]
    fn writing_keeps_comments_and_unknown_fields() {
        let mut list = TaskList::parse(
            "# Small things.\nformat = 1\nowner = \"me\"\n\n\
             # Urgent!\n[[task]]\nid = \"aaaa\"\ntitle = 'Call' # by phone\nstatus = \"open\"\nnote = \"x\"\n\n\
             [[task]]\nid = \"bbbb\"\ntitle = \"B\"\nstatus = \"open\"\n",
        )
        .unwrap();
        list.move_task(&id("bbbb"), 0).unwrap();
        list.set_title(&id("aaaa"), "Call Kim").unwrap();
        assert_eq!(
            list.to_toml(),
            "# Small things.\nformat = 1\nowner = \"me\"\n\n\
             [[task]]\nid = \"bbbb\"\ntitle = \"B\"\nstatus = \"open\"\n\n\
             # Urgent!\n[[task]]\nid = \"aaaa\"\ntitle = \"Call Kim\" # by phone\nstatus = \"open\"\nnote = \"x\"\n"
        );
    }

    #[test]
    fn inline_tables_are_read() {
        let list = TaskList::parse(
            "format = 1\ntask = [{ id = \"aaaa\", title = \"A\" }, { id = \"bbbb\", title = \"B\", status = \"done\" }]\n",
        )
        .unwrap();
        assert_eq!(ids(&list), ["aaaa", "bbbb"]);
        let written = TaskList::parse(&list.to_toml()).unwrap();
        assert_eq!(ids(&written), ["aaaa", "bbbb"]);
    }

    #[test]
    fn archive_parts() {
        let mut list = sample();
        let finished = list.take_finished();
        assert_eq!(ids(&list), ["t9x2", "h4c8"]);

        let mut archive = TaskList::default();
        archive.push_finished(finished[0].clone());
        archive.push_finished(finished[0].clone());
        let ids = ids(&archive);
        assert_eq!(ids[0], "r3m7");
        assert_ne!(ids[1], "r3m7");
    }

    #[test]
    fn invalid() {
        for text in [
            "",
            "format = 2",
            "format = 1\n[[task]]\ntitle = \"No id\"",
            "format = 1\n[[task]]\nid = \"aaaa\"",
            "format = 1\n[[task]]\nid = \"AAAA\"\ntitle = \"A\"",
            "format = 1\n[[task]]\nid = \"aaaa\"\ntitle = \"A\"\nstatus = \"waiting\"",
            "format = 1\n[[task]]\nid = \"aaaa\"\ntitle = \"A\"\ndue = \"2026-10-01\"",
            "format = 1\n[[task]]\nid = \"aaaa\"\ntitle = \"A\"\n[[task]]\nid = \"aaaa\"\ntitle = \"B\"",
            "format = 1\ntask = 3",
        ] {
            assert!(TaskList::parse(text).is_err(), "{text:?}");
        }
    }
}
