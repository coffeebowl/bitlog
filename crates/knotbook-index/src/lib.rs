//! Local SQLite index of a Knotbook vault, for searching and statistics.
//!
//! The index lives in `.knotbook/index.sqlite` inside the vault and is never
//! synced. The vault files stay the only source of truth: the index can be
//! deleted and rebuilt from them at any time.

mod content;

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::NaiveDate;
use knotbook_core::{NotePath, ReadError, Vault, VaultChange, content_hash};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use rusqlite_migration::{M, Migrations};
use thiserror::Error;

use crate::content::{insert, remove, sync_projects};

/// The schema, one step per migration. Add steps, never change them.
const MIGRATION_STEPS: &[M] = &[
    M::up(include_str!("migrations/01-content.sql")),
    M::up(include_str!("migrations/02-files.sql")),
];
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
            "DELETE FROM files; DELETE FROM blocks; DELETE FROM days; DELETE FROM notes;
             DELETE FROM projects; DELETE FROM tasks;",
        )?;
        let skipped = refresh(&tx, vault)?;
        tx.commit()?;
        Ok(skipped)
    }

    /// Brings the index up to date with the files of `vault`, reading only
    /// those that changed since they were last read. It looks at every file,
    /// so it also finds changes made while nobody was watching, and those
    /// saved by this program, which watching leaves out.
    ///
    /// Files that cannot be read are taken out of the index and returned.
    pub fn refresh(&mut self, vault: &Vault) -> Result<Vec<ReadError>, IndexError> {
        let tx = self.connection.transaction()?;
        let skipped = refresh(&tx, vault)?;
        tx.commit()?;
        Ok(skipped)
    }

    /// Brings the index up to date with the files named in `changes`, as
    /// reported by [`Vault::watch`]. `vault` has to be up to date with
    /// changes to projects.
    ///
    /// Files that cannot be read are taken out of the index and returned.
    pub fn apply(
        &mut self,
        vault: &Vault,
        changes: &[VaultChange],
    ) -> Result<Vec<ReadError>, IndexError> {
        let tx = self.connection.transaction()?;
        let mut skipped = Vec::new();
        for change in changes {
            if let VaultChange::Project(_) = change {
                sync_projects(&tx, vault)?;
            } else if let Some(file) = IndexFile::from_change(change) {
                skipped.extend(update(&tx, vault, &file)?);
            }
        }
        tx.commit()?;
        Ok(skipped)
    }
}

/// A vault file whose content goes into the index. Project files are not
/// among them: the vault holds the projects read already.
#[derive(Debug)]
enum IndexFile {
    Day(NaiveDate),
    Note(NotePath),
    Tasks,
}

impl IndexFile {
    fn path(&self, vault: &Vault) -> PathBuf {
        match self {
            Self::Day(date) => vault.day_path(*date),
            Self::Note(note) => vault.note_path(note),
            Self::Tasks => vault.tasks_path(),
        }
    }

    /// The key of the file in the table `files`.
    fn key(&self, vault: &Vault) -> String {
        self.path(vault)
            .strip_prefix(vault.root())
            .expect("vault files lie in the vault")
            .to_str()
            .expect("the names of indexed files are UTF-8")
            .to_owned()
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::from_change(&VaultChange::from_path(Path::new(key))?)
    }

    fn from_change(change: &VaultChange) -> Option<Self> {
        match change {
            VaultChange::Day(date) => Some(Self::Day(*date)),
            VaultChange::Note(note) => Some(Self::Note(note.clone())),
            VaultChange::Tasks => Some(Self::Tasks),
            VaultChange::Config | VaultChange::Project(_) => None,
        }
    }
}

/// What the table `files` holds about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileState {
    modified: i64,
    size: i64,
    hash: i64,
}

fn refresh(tx: &Transaction, vault: &Vault) -> Result<Vec<ReadError>, IndexError> {
    sync_projects(tx, vault)?;
    let mut files = vec![IndexFile::Tasks];
    files.extend(vault.all_days()?.into_iter().map(IndexFile::Day));
    for project in vault.projects() {
        files.extend(vault.notes(&project.slug)?.into_iter().map(IndexFile::Note));
    }
    // Files read before that are gone now.
    let current: HashSet<String> = files.iter().map(|file| file.key(vault)).collect();
    let known: Vec<String> = tx
        .prepare("SELECT path FROM files")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for key in known.iter().filter(|key| !current.contains(*key)) {
        match IndexFile::from_key(key) {
            Some(file) => files.push(file),
            None => forget(tx, key)?,
        }
    }
    let mut skipped = Vec::new();
    for file in &files {
        skipped.extend(update(tx, vault, file)?);
    }
    Ok(skipped)
}

/// Reads `file` again if it changed since it was last read, or takes it out
/// of the index if it is gone. Returns why it cannot be read, if it cannot.
fn update(
    tx: &Transaction,
    vault: &Vault,
    file: &IndexFile,
) -> Result<Option<ReadError>, IndexError> {
    let path = file.path(vault);
    let key = file.key(vault);
    let known = tx
        .prepare_cached("SELECT modified, size, hash FROM files WHERE path = ?")?
        .query_row([&key], |row| {
            Ok(FileState {
                modified: row.get(0)?,
                size: row.get(1)?,
                hash: row.get(2)?,
            })
        })
        .optional()?;
    // A note of a project that is gone is gone as well.
    let orphan = matches!(file, IndexFile::Note(note) if vault.project(note.project()).is_none());
    let metadata = match fs::metadata(&path) {
        Ok(_) if orphan => None,
        Ok(metadata) => Some(metadata),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return drop_file(tx, file, &key, Some(ReadError::Io { path, source })),
    };
    let Some(metadata) = metadata else {
        return drop_file(tx, file, &key, None);
    };
    let (modified, size) = stamp(&metadata);
    if known.is_some_and(|known| (known.modified, known.size) == (modified, size)) {
        return Ok(None);
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) => return drop_file(tx, file, &key, Some(ReadError::Io { path, source })),
    };
    // Stored as SQLite's signed integer, bit for bit.
    let hash = content_hash(&text) as i64;
    // Sync tools often touch files without changing them.
    if !known.is_some_and(|known| known.hash == hash) {
        remove(tx, file)?;
        match insert(tx, vault, file) {
            Ok(()) => {}
            Err(IndexError::Read(err)) => return drop_file(tx, file, &key, Some(err)),
            Err(err) => return Err(err),
        }
    }
    tx.prepare_cached(
        "INSERT INTO files (path, modified, size, hash) VALUES (?, ?, ?, ?)
         ON CONFLICT (path) DO UPDATE SET
             modified = excluded.modified, size = excluded.size, hash = excluded.hash",
    )?
    .execute(params![key, modified, size, hash])?;
    Ok(None)
}

/// Takes `file` out of the index, so that it is read again next time.
fn drop_file(
    tx: &Transaction,
    file: &IndexFile,
    key: &str,
    err: Option<ReadError>,
) -> Result<Option<ReadError>, IndexError> {
    remove(tx, file)?;
    forget(tx, key)?;
    Ok(err)
}

fn forget(tx: &Transaction, key: &str) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM files WHERE path = ?", [key])?;
    Ok(())
}

/// Modification time in nanoseconds since 1970 and size of a file.
fn stamp(metadata: &Metadata) -> (i64, i64) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos() as i64);
    (modified, metadata.len() as i64)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

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

    /// A copy of the sample vault that tests may change, with its index.
    fn sample_copy() -> (TempDir, Vault, Index) {
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
        copy(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"),
            &dir.0,
        );
        let vault = Vault::open(&dir.0).unwrap();
        let mut index = Index::open(&vault).unwrap();
        index.rebuild(&vault).unwrap();
        (dir, vault, index)
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    fn block_title(index: &Index, id: &str) -> Option<String> {
        index
            .connection
            .query_row("SELECT title FROM blocks WHERE id = ?", [id], |row| {
                row.get(0)
            })
            .optional()
            .unwrap()
    }

    /// Changes the title of a block in the index only, to see whether the
    /// index reads the day file again.
    fn tamper(index: &Index, id: &str) {
        index
            .connection
            .execute("UPDATE blocks SET title = 'stale' WHERE id = ?", [id])
            .unwrap();
    }

    fn append(path: &Path, text: &str) {
        let old = fs::read_to_string(path).unwrap();
        fs::write(path, old + text).unwrap();
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

    #[test]
    fn refresh_reads_only_changed_files() {
        let (_dir, vault, mut index) = sample_copy();
        tamper(&index, "ff66");
        tamper(&index, "a1b2");
        append(&vault.day_path(date(23)), "Rolled back at 00:20.\n");
        assert!(index.refresh(&vault).unwrap().is_empty());
        assert_eq!(block_title(&index, "ff66").unwrap(), "Release deployment");
        assert_eq!(block_title(&index, "a1b2").unwrap(), "stale");
    }

    #[test]
    fn refresh_skips_touched_files() {
        let (_dir, vault, mut index) = sample_copy();
        tamper(&index, "ff66");
        let path = vault.day_path(date(23));
        let later = SystemTime::now() + Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        index.refresh(&vault).unwrap();
        assert_eq!(block_title(&index, "ff66").unwrap(), "stale");
        let modified: i64 = index
            .connection
            .query_row(
                "SELECT modified FROM files WHERE path = 'daily/2026/09/2026-09-23.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(modified, stamp(&fs::metadata(&path).unwrap()).0);
    }

    #[test]
    fn refresh_follows_new_and_removed_files() {
        let (dir, vault, mut index) = sample_copy();
        fs::remove_file(vault.day_path(date(21))).unwrap();
        fs::remove_file(dir.0.join("projects/infra/notes/deployment.md")).unwrap();
        fs::write(dir.0.join("projects/infra/notes/backups.md"), "# Backups\n").unwrap();
        fs::remove_file(vault.tasks_path()).unwrap();
        index.refresh(&vault).unwrap();
        assert_eq!(count(&index, "days"), 2);
        assert_eq!(block_title(&index, "a1b2"), None);
        let notes: Vec<String> = index
            .connection
            .prepare("SELECT name FROM notes WHERE project = 'infra'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(notes, ["backups"]);
        assert_eq!(count(&index, "tasks"), 0);
        // Two days and three notes.
        assert_eq!(count(&index, "files"), 5);
    }

    #[test]
    fn refresh_removes_the_notes_of_removed_projects() {
        let (dir, _vault, mut index) = sample_copy();
        let project = dir.0.join("projects/webshop/project.toml");
        let text = fs::read_to_string(&project).unwrap();
        fs::remove_file(&project).unwrap();
        index.refresh(&Vault::open(&dir.0).unwrap()).unwrap();
        assert_eq!(count(&index, "projects"), 4);
        assert_eq!(count(&index, "notes"), 1);
        // Blocks may belong to projects that do not exist.
        assert_eq!(count(&index, "blocks"), 17);
        fs::write(&project, text).unwrap();
        index.refresh(&Vault::open(&dir.0).unwrap()).unwrap();
        assert_eq!(count(&index, "notes"), 3);
    }

    #[test]
    fn refresh_drops_files_that_break() {
        let (_dir, vault, mut index) = sample_copy();
        let path = vault.day_path(date(23));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, "no front matter").unwrap();
        let skipped = index.refresh(&vault).unwrap();
        assert!(
            matches!(&skipped[..], [ReadError::Invalid { path: broken, .. }] if *broken == path),
            "{skipped:?}"
        );
        assert_eq!(block_title(&index, "ff66"), None);
        // Reported again until repaired.
        assert_eq!(index.refresh(&vault).unwrap().len(), 1);
        fs::write(&path, text).unwrap();
        assert!(index.refresh(&vault).unwrap().is_empty());
        assert_eq!(count(&index, "blocks"), 17);
    }

    #[test]
    fn apply_reads_the_changed_files() {
        let (dir, vault, mut index) = sample_copy();
        tamper(&index, "ff66");
        tamper(&index, "a1b2");
        append(&vault.day_path(date(23)), "Rolled back at 00:20.\n");
        fs::remove_file(vault.day_path(date(22))).unwrap();
        let changes = [VaultChange::Day(date(22)), VaultChange::Day(date(23))];
        assert!(index.apply(&vault, &changes).unwrap().is_empty());
        assert_eq!(block_title(&index, "ff66").unwrap(), "Release deployment");
        assert_eq!(block_title(&index, "a1b2").unwrap(), "stale");
        assert_eq!(count(&index, "days"), 2);

        let project = dir.0.join("projects/webshop/project.toml");
        let text = fs::read_to_string(&project).unwrap();
        fs::write(
            &project,
            text.replace("name = \"Webshop\"", "name = \"Shop\""),
        )
        .unwrap();
        let slug = "webshop".parse().unwrap();
        index
            .apply(&Vault::open(&dir.0).unwrap(), &[VaultChange::Project(slug)])
            .unwrap();
        let name: String = index
            .connection
            .query_row(
                "SELECT name FROM projects WHERE slug = 'webshop'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Shop");
        assert_eq!(count(&index, "notes"), 3);
    }
}
