//! Commands for the global task list.

use anyhow::Result;
use bitlog_core::{EditError, Task, TaskId, TaskList, TaskStatus, Vault};
use chrono::NaiveDate;

use crate::table::table;

/// The open tasks with their ids and due dates, and with `all` the
/// finished ones below.
pub fn format_tasks(tasks: &TaskList, all: bool, today: NaiveDate) -> String {
    let (open, finished): (Vec<&Task>, Vec<&Task>) =
        tasks.tasks().iter().partition(|task| task.is_open());
    let mut text = if open.is_empty() {
        "No open tasks.\n".to_owned()
    } else {
        lines(&open, |task| match task.due {
            Some(due) if due < today => format!("due {due}, overdue"),
            Some(due) => format!("due {due}"),
            None => String::new(),
        })
    };
    if all && !finished.is_empty() {
        text.push_str("\nFinished:\n");
        text.push_str(&lines(&finished, |task| {
            let status = task.status.as_str();
            match task.done {
                Some(done) => format!("{status} {done}"),
                None => status.to_owned(),
            }
        }));
    }
    text
}

/// One line per task: id, title and `note`, with the titles aligned.
fn lines(tasks: &[&Task], note: impl Fn(&Task) -> String) -> String {
    let rows: Vec<[String; 3]> = tasks
        .iter()
        .map(|task| [task.id.to_string(), task.title.clone(), note(task)])
        .collect();
    table(&rows, &[])
}

/// Applies `change` to the task list and saves it.
fn update(
    vault: &Vault,
    change: impl FnOnce(&mut TaskList) -> Result<(), EditError>,
) -> Result<TaskList> {
    let tasks = vault.load_tasks()?;
    Ok(vault.update_tasks(&tasks, change)?)
}

pub fn add_task(
    vault: &Vault,
    title: &str,
    due: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<()> {
    let mut id = None;
    update(vault, |tasks| {
        let added = tasks.add(title, today)?;
        tasks.set_due(&added, due)?;
        id = Some(added);
        Ok(())
    })?;
    println!("Added task {}", id.expect("the task was added"));
    Ok(())
}

pub fn set_status(vault: &Vault, id: &TaskId, status: TaskStatus, today: NaiveDate) -> Result<()> {
    update(vault, |tasks| tasks.set_status(id, status, today))?;
    Ok(())
}

/// Changes to a task; `None` keeps a value as it is.
pub struct TaskChanges {
    pub title: Option<String>,
    /// `Some(None)` removes the due date.
    pub due: Option<Option<NaiveDate>>,
}

pub fn edit_task(vault: &Vault, id: &TaskId, changes: TaskChanges) -> Result<()> {
    update(vault, |tasks| {
        if let Some(title) = &changes.title {
            tasks.set_title(id, title)?;
        }
        if let Some(due) = changes.due {
            tasks.set_due(id, due)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// Moves the task `id` to `position`, counted from 1 among the open or the
/// finished tasks, whichever it belongs to.
pub fn move_task(vault: &Vault, id: &TaskId, position: usize) -> Result<()> {
    update(vault, |tasks| {
        let task = tasks
            .task(id)
            .ok_or_else(|| EditError::UnknownTask(id.clone()))?;
        let first = if task.is_open() {
            0
        } else {
            tasks.tasks().iter().filter(|task| task.is_open()).count()
        };
        tasks.move_task(id, first + position.saturating_sub(1))
    })?;
    Ok(())
}

pub fn archive_tasks(vault: &Vault, today: NaiveDate) -> Result<()> {
    let tasks = vault.load_tasks()?;
    let finished = tasks.tasks().iter().filter(|task| !task.is_open()).count();
    vault.archive_tasks(&tasks, today)?;
    println!("Archived {finished} finished tasks.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn sample_tasks() -> TaskList {
        Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
            .unwrap()
            .load_tasks()
            .unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn open_tasks() {
        assert_eq!(
            format_tasks(&sample_tasks(), false, date(2026, 9, 24)),
            "t9x2  Get back to Kim about the handover\n\
             h4c8  Renew the TLS certificate for staging  due 2026-09-30\n"
        );
    }

    #[test]
    fn all_tasks() {
        assert_eq!(
            format_tasks(&sample_tasks(), true, date(2026, 10, 1)),
            "t9x2  Get back to Kim about the handover\n\
             h4c8  Renew the TLS certificate for staging  due 2026-09-30, overdue\n\
             \n\
             Finished:\n\
             r3m7  Take the keyboard to the office    done 2026-09-22\n\
             w5d1  Book a meeting room for the retro  dropped\n"
        );
    }

    #[test]
    fn no_tasks() {
        assert_eq!(
            format_tasks(&TaskList::default(), true, date(2026, 9, 24)),
            "No open tasks.\n"
        );
    }
}
