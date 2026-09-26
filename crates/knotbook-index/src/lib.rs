//! Local SQLite index of a Knotbook vault, for searching and statistics.
//!
//! The index lives in `.knotbook/index.sqlite` inside the vault and is never
//! synced. The vault files stay the only source of truth: the index can be
//! deleted and rebuilt from them at any time.

use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::{NaiveTime, Timelike};
use knotbook_core::{Day, NoteFile, Project, ReadError, TaskList, Vault};
use rusqlite::{Connection, Transaction, params};
use rusqlite_migration::{M, Migrations};
use thiserror::Error;

/// The schema, one step per migration. Add steps, never change them.
const MIGRATION_STEPS: &[M] = &[M::up(include_str!("migrations/01-content.sql"))];
const MIGRATIONS: Migrations = Migrations::from_slice(MIGRATION_STEPS);

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("cannot create {path}: {source}")]
    Folder { path: PathBuf, source: io::Error },
    #[error("index database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("cannot update the index schema: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Read(#[from] ReadError),
}

pub struct Index {
    connection: Connection,
}

impl Index {
    /// Where the index of `vault` lives.
    pub fn path(vault: &Vault) -> PathBuf {
        vault.root().join(".knotbook").join("index.sqlite")
    }

    /// Opens the index of `vault`, creating it if needed, and brings its
    /// schema up to date.
    pub fn open(vault: &Vault) -> Result<Self, IndexError> {
        let path = Self::path(vault);
        let folder = path.parent().expect("the index lies in a folder");
        fs::create_dir_all(folder).map_err(|source| IndexError::Folder {
            path: folder.to_owned(),
            source,
        })?;
        Self::with_connection(Connection::open(&path)?)
    }

    fn with_connection(mut connection: Connection) -> Result<Self, IndexError> {
        connection.pragma_update(None, "foreign_keys", true)?;
        MIGRATIONS.to_latest(&mut connection)?;
        Ok(Self { connection })
    }

    /// Replaces the whole content of the index with what the files of
    /// `vault` hold now.
    ///
    /// Day files, notes and the task list that cannot be read are left out
    /// and returned, so that one broken file does not stop the rest.
    pub fn rebuild(&mut self, vault: &Vault) -> Result<Vec<ReadError>, IndexError> {
        let tx = self.connection.transaction()?;
        tx.execute_batch(
            "DELETE FROM blocks; DELETE FROM days; DELETE FROM notes;
             DELETE FROM projects; DELETE FROM tasks;",
        )?;
        let mut skipped = Vec::new();
        for project in vault.projects() {
            insert_project(&tx, project)?;
        }
        for date in vault.all_days()? {
            match vault.load_day(date) {
                Ok(Some(file)) => insert_day(&tx, &file.day)?,
                Ok(None) => {}
                Err(err) => skipped.push(err),
            }
        }
        for project in vault.projects() {
            for note in vault.notes(&project.slug)? {
                match vault.load_note(&note) {
                    Ok(file) => insert_note(&tx, &file)?,
                    Err(err) => skipped.push(err),
                }
            }
        }
        match vault.load_tasks() {
            Ok(tasks) => insert_tasks(&tx, &tasks)?,
            Err(err) => skipped.push(err),
        }
        tx.commit()?;
        Ok(skipped)
    }
}

fn insert_project(tx: &Transaction, project: &Project) -> rusqlite::Result<()> {
    tx.prepare_cached(
        "INSERT INTO projects (slug, name, color, category, status, pinned, created)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )?
    .execute(params![
        project.slug.as_str(),
        project.name,
        project.color,
        project.category,
        project.status.as_str(),
        project.pinned,
        project.created,
    ])?;
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
        day.work_start.map(minutes),
        day.work_end.map(minutes),
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
    Ok(())
}

fn insert_note(tx: &Transaction, file: &NoteFile) -> rusqlite::Result<()> {
    tx.prepare_cached("INSERT INTO notes (project, name, text) VALUES (?, ?, ?)")?
        .execute(params![
            file.path.project().as_str(),
            file.path.name(),
            file.text,
        ])?;
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

fn minutes(time: NaiveTime) -> u32 {
    time.num_seconds_from_midnight() / 60
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::NaiveDate;

    use super::*;

    fn sample() -> Vault {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault");
        Vault::open(&path).unwrap()
    }

    /// An index in memory, so that tests leave the sample vault alone.
    fn in_memory() -> Index {
        Index::with_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn count(index: &Index, table: &str) -> i64 {
        index
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// A folder that is deleted again when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let name = format!("knotbook-index-{:016x}", fastrand::u64(..));
            Self(std::env::temp_dir().join(name))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[test]
    fn rebuild_reads_the_whole_vault() {
        let mut index = in_memory();
        let skipped = index.rebuild(&sample()).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(count(&index, "projects"), 5);
        // The sync conflict copy is not a day.
        assert_eq!(count(&index, "days"), 3);
        assert_eq!(count(&index, "blocks"), 17);
        assert_eq!(count(&index, "notes"), 3);
        assert_eq!(count(&index, "tasks"), 4);
    }

    #[test]
    fn rebuild_stores_blocks_in_minutes() {
        let mut index = in_memory();
        index.rebuild(&sample()).unwrap();
        let (project, start, end): (String, u32, u32) = index
            .connection
            .query_row(
                "SELECT project, start_minute, end_minute FROM blocks
                 WHERE date = '2026-09-23' AND id = 'ff66'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (project.as_str(), start, end),
            ("infra", 22 * 60 + 30, 24 * 60 + 30)
        );
    }

    #[test]
    fn rebuild_replaces_the_old_content() {
        let mut index = in_memory();
        index.rebuild(&sample()).unwrap();
        index.rebuild(&sample()).unwrap();
        assert_eq!(count(&index, "days"), 3);
        assert_eq!(count(&index, "blocks"), 17);
    }

    #[test]
    fn open_creates_the_index_in_the_vault() {
        let dir = TempDir::new();
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let vault = Vault::create(&dir.0, "Test", today).unwrap();
        let mut index = Index::open(&vault).unwrap();
        index.rebuild(&vault).unwrap();
        assert!(dir.0.join(".knotbook/index.sqlite").is_file());
        // Opening again finds the schema up to date.
        drop(index);
        let index = Index::open(&vault).unwrap();
        assert_eq!(count(&index, "projects"), vault.projects().len() as i64);
    }

    #[test]
    fn rebuild_skips_unreadable_files() {
        let dir = TempDir::new();
        let today = NaiveDate::from_ymd_opt(2026, 9, 26).unwrap();
        let vault = Vault::create(&dir.0, "Test", today).unwrap();
        let broken = dir.0.join("daily/2026/09/2026-09-24.md");
        fs::create_dir_all(broken.parent().unwrap()).unwrap();
        fs::write(&broken, "---\nformat: [\n---\n").unwrap();
        let mut index = Index::open(&vault).unwrap();
        let skipped = index.rebuild(&vault).unwrap();
        assert!(
            matches!(&skipped[..], [ReadError::Invalid { path, .. }] if *path == broken),
            "{skipped:?}"
        );
        assert_eq!(count(&index, "days"), 0);
        assert_eq!(count(&index, "projects"), vault.projects().len() as i64);
    }
}
