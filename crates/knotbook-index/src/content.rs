//! Putting the content of vault files into the index and taking it out.

use std::collections::HashSet;

use knotbook_core::{Day, NotePath, ProjectSlug, TaskList, Vault, minute_of_day, wiki_links};
use rusqlite::{Transaction, params};

use crate::{Found, IndexError, IndexFile};

/// Makes the projects in the index those of `vault`. Removing a project
/// removes its notes, too.
pub(crate) fn sync_projects(tx: &Transaction, vault: &Vault) -> rusqlite::Result<()> {
    // An upsert rather than `INSERT OR REPLACE`, which would delete the row
    // first and with it the project's notes.
    let mut upsert = tx.prepare_cached(
        "INSERT INTO projects (slug, name, color, category, status, pinned, created)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (slug) DO UPDATE SET name = excluded.name, color = excluded.color,
             category = excluded.category, status = excluded.status,
             pinned = excluded.pinned, created = excluded.created",
    )?;
    for project in vault.projects() {
        upsert.execute(params![
            project.slug.as_str(),
            project.name,
            project.color,
            project.category,
            project.status.as_str(),
            project.pinned,
            project.created,
        ])?;
    }
    let current: HashSet<&str> = vault
        .projects()
        .iter()
        .map(|project| project.slug.as_str())
        .collect();
    let indexed: Vec<String> = tx
        .prepare("SELECT slug FROM projects")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for slug in indexed
        .iter()
        .filter(|slug| !current.contains(slug.as_str()))
    {
        tx.execute("DELETE FROM projects WHERE slug = ?", [slug])?;
        // Slugs hold no `%` or `_`.
        tx.execute(
            "DELETE FROM files WHERE path LIKE 'projects/' || ? || '/%'",
            [slug],
        )?;
    }
    Ok(())
}

/// Reads `file` from `vault` and puts its content into the index. A file
/// that is gone adds nothing.
pub(crate) fn insert(tx: &Transaction, vault: &Vault, file: &IndexFile) -> Result<(), IndexError> {
    match file {
        IndexFile::Day(date) => {
            if let Some(file) = vault.load_day(*date)? {
                insert_day(tx, &file.day)?;
            }
        }
        IndexFile::Note(note) => insert_note(tx, note, &vault.load_note(note)?.text)?,
        IndexFile::Tasks => insert_tasks(tx, &vault.load_tasks()?)?,
    }
    Ok(())
}

/// Takes the content of `file` out of the index.
pub(crate) fn remove(tx: &Transaction, file: &IndexFile) -> rusqlite::Result<()> {
    match file {
        IndexFile::Day(date) => tx.execute("DELETE FROM days WHERE date = ?", [date])?,
        IndexFile::Note(note) => tx.execute(
            "DELETE FROM notes WHERE project = ? AND name = ?",
            [note.project().as_str(), note.name()],
        )?,
        IndexFile::Tasks => tx.execute("DELETE FROM tasks", [])?,
    };
    Ok(())
}

fn insert_day(tx: &Transaction, day: &Day) -> rusqlite::Result<()> {
    tx.prepare_cached(
        "INSERT INTO days (date, kind, location, work_start, work_end, note)
         VALUES (?, ?, ?, ?, ?, ?)",
    )?
    .execute(params![
        day.date,
        day.kind,
        day.location.as_ref().map(|key| key.as_str()),
        day.work_start.map(minute_of_day),
        day.work_end.map(minute_of_day),
        day.note,
    ])?;
    let mut insert_block = tx.prepare_cached(
        "INSERT INTO blocks (date, id, project, start_minute, end_minute, title, text)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )?;
    for block in &day.blocks {
        let (start, end) = block.span();
        insert_block.execute(params![
            day.date,
            block.id.as_str(),
            block.project.as_str(),
            start,
            end,
            block.title,
            block.text,
        ])?;
    }
    insert_links(tx, &Found::DayNote(day.date), &day.note, None)?;
    for block in &day.blocks {
        let source = Found::Block {
            date: day.date,
            id: block.id.clone(),
        };
        insert_links(tx, &source, &block.text, Some(&block.project))?;
    }
    Ok(())
}

fn insert_note(tx: &Transaction, note: &NotePath, text: &str) -> rusqlite::Result<()> {
    tx.prepare_cached("INSERT INTO notes (project, name, text) VALUES (?, ?, ?)")?
        .execute(params![note.project().as_str(), note.name(), text])?;
    insert_links(tx, &Found::Note(note.clone()), text, Some(note.project()))
}

/// Puts the wiki links in `text`, found in `source` of the project
/// `project`, into the index. Links that point nowhere are left out.
fn insert_links(
    tx: &Transaction,
    source: &Found,
    text: &str,
    project: Option<&ProjectSlug>,
) -> rusqlite::Result<()> {
    let (note, date, block) = match source {
        Found::Note(note) => (Some(note), None, None),
        Found::DayNote(date) => (None, Some(*date), None),
        Found::Block { date, id } => (None, Some(*date), Some(id.as_str())),
        Found::Task(_) => unreachable!("tasks hold no links"),
    };
    let mut insert = tx.prepare_cached(
        "INSERT INTO links (note_project, note_name, date, block, target_project, target_name)
         VALUES (?, ?, ?, ?, ?, ?)",
    )?;
    for target in wiki_links(text, project)
        .into_iter()
        .filter_map(|link| link.note)
    {
        insert.execute(params![
            note.map(|note| note.project().as_str()),
            note.map(|note| note.name()),
            date,
            block,
            target.project().as_str(),
            target.name(),
        ])?;
    }
    Ok(())
}

fn insert_tasks(tx: &Transaction, tasks: &TaskList) -> rusqlite::Result<()> {
    let mut insert = tx.prepare_cached(
        "INSERT INTO tasks (id, title, status, created, due, done) VALUES (?, ?, ?, ?, ?, ?)",
    )?;
    for task in tasks.tasks() {
        insert.execute(params![
            task.id.as_str(),
            task.title,
            task.status.as_str(),
            task.created,
            task.due,
            task.done,
        ])?;
    }
    Ok(())
}
