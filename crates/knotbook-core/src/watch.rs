//! Watching a vault for changes made elsewhere.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{fmt, fs, thread};

use chrono::{Datelike, NaiveDate};
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;

use crate::conflict::copy_of;
use crate::file::content_hash;
use crate::vault::day_file_date;
use crate::{NotePath, ProjectSlug};

/// Long enough to see a sync tool's burst of writes as one change.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// A vault file that was changed, created or removed, named by what it holds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VaultChange {
    Config,
    Tasks,
    Project(ProjectSlug),
    Note(NotePath),
    Day(NaiveDate),
}

#[derive(Debug, Error)]
#[error("cannot watch {path}: {source}")]
pub struct WatchError {
    pub path: PathBuf,
    pub source: notify::Error,
}

/// Watches a vault until it is dropped.
pub struct VaultWatcher {
    _watcher: RecommendedWatcher,
}

impl fmt::Debug for VaultWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultWatcher").finish_non_exhaustive()
    }
}

/// The content this program last wrote to each file, by path relative to the
/// vault, or `None` if it removed the file, so that watching can tell its own
/// changes apart.
#[derive(Debug, Clone, Default)]
pub(crate) struct OwnWrites(Arc<Mutex<HashMap<PathBuf, Option<u64>>>>);

impl OwnWrites {
    pub fn record(&self, relative: &Path, text: Option<&str>) {
        self.lock()
            .insert(relative.to_owned(), text.map(content_hash));
    }

    /// Whether `text`, the content of the file now (`None` if there is
    /// none), is what this program left there last.
    fn is_own(&self, relative: &Path, text: Option<&str>) -> bool {
        self.lock().get(relative) == Some(&text.map(content_hash))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Option<u64>>> {
        self.0
            .lock()
            .expect("nothing panics while holding the lock")
    }
}

/// Starts watching the vault in `root`. `on_change` is called on a thread
/// of its own with the files changed elsewhere, sorted and without
/// duplicates; files whose content is what `own_writes` holds are left out.
pub(crate) fn watch(
    root: &Path,
    own_writes: OwnWrites,
    on_change: impl Fn(Result<Vec<VaultChange>, WatchError>) + Send + 'static,
) -> Result<VaultWatcher, WatchError> {
    let error = |source| WatchError {
        path: root.to_owned(),
        source,
    };
    // Events name absolute, resolved paths.
    let root = fs::canonicalize(root).map_err(|err| error(err.into()))?;
    let (sender, events) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(sender).map_err(error)?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(error)?;
    // Ends when the watcher, and with it the sender, is dropped.
    thread::spawn(move || {
        while let Some(burst) = next_burst(&events) {
            let paths = match burst {
                Ok(paths) => paths,
                Err(source) => {
                    on_change(Err(WatchError {
                        path: root.clone(),
                        source,
                    }));
                    continue;
                }
            };
            let changes: BTreeSet<VaultChange> = paths
                .iter()
                .filter_map(|path| {
                    let relative = path.strip_prefix(&root).ok()?;
                    // A conflict copy changes what there is to merge into
                    // its original.
                    let change = VaultChange::from_path(relative).or_else(|| copy_of(relative))?;
                    let text = fs::read_to_string(path).ok();
                    (!own_writes.is_own(relative, text.as_deref())).then_some(change)
                })
                .collect();
            if !changes.is_empty() {
                on_change(Ok(changes.into_iter().collect()));
            }
        }
    });
    Ok(VaultWatcher { _watcher: watcher })
}

/// The files changed next, collected until no change comes for
/// `DEBOUNCE`, or the first error. `None` once the watcher is gone.
fn next_burst(
    events: &mpsc::Receiver<notify::Result<notify::Event>>,
) -> Option<Result<BTreeSet<PathBuf>, notify::Error>> {
    let mut paths = BTreeSet::new();
    loop {
        let event = if paths.is_empty() {
            events.recv().ok()?
        } else {
            match events.recv_timeout(DEBOUNCE) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => return Some(Ok(paths)),
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        };
        match event {
            Ok(event) if changes_files(event.kind) => paths.extend(event.paths),
            Ok(_) => {}
            Err(err) => return Some(Err(err)),
        }
    }
}

/// Whether an event of `kind` can mean new content. Opening and reading
/// a file, as this program does after every change, does not: counting
/// that would reload the vault again and again.
fn changes_files(kind: EventKind) -> bool {
    match kind {
        EventKind::Access(access) => access == AccessKind::Close(AccessMode::Write),
        _ => true,
    }
}

impl VaultChange {
    /// What the file at `relative`, a path inside the vault, holds. `None`
    /// for all files Knotbook does not read, such as temporary files.
    pub fn from_path(relative: &Path) -> Option<Self> {
        let parts: Vec<&str> = relative
            .components()
            .map(|component| match component {
                Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect::<Option<_>>()?;
        match parts.as_slice() {
            ["knotbook.toml"] => Some(VaultChange::Config),
            ["tasks.toml"] => Some(VaultChange::Tasks),
            ["projects", slug, "project.toml"] => slug.parse().ok().map(VaultChange::Project),
            ["projects", slug, "notes", name] => {
                NotePath::new(slug.parse().ok()?, name.strip_suffix(".md")?)
                    .ok()
                    .map(VaultChange::Note)
            }
            ["daily", year, month, name] => day_file_date(name)
                .filter(|date| {
                    *year == format!("{:04}", date.year())
                        && *month == format!("{:02}", date.month())
                })
                .map(VaultChange::Day),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_changes_nothing() {
        use notify::event::{CreateKind, ModifyKind};
        assert!(!changes_files(EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!changes_files(EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(changes_files(EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(changes_files(EventKind::Modify(ModifyKind::Any)));
        assert!(changes_files(EventKind::Create(CreateKind::File)));
    }

    #[test]
    fn changes_of_paths() {
        let change = |path: &str| VaultChange::from_path(Path::new(path));
        assert_eq!(change("knotbook.toml"), Some(VaultChange::Config));
        assert_eq!(change("tasks.toml"), Some(VaultChange::Tasks));
        assert_eq!(
            change("projects/infra/project.toml"),
            Some(VaultChange::Project("infra".parse().unwrap()))
        );
        assert_eq!(
            change("projects/infra/notes/Deployment steps.md"),
            Some(VaultChange::Note(
                "projects/infra/notes/Deployment steps.md".parse().unwrap()
            ))
        );
        assert_eq!(
            change("daily/2026/09/2026-09-21.md"),
            Some(VaultChange::Day(
                NaiveDate::from_ymd_opt(2026, 9, 21).unwrap()
            ))
        );
        for ignored in [
            "daily/2026/09/.2026-09-21.md.0badf00d.tmp",
            "daily/2026/09/2026-09-22.sync-conflict-20260922-181530-KNOTBK7.md",
            "projects/infra/notes/deployment (conflicted copy 2026-09-22 181530).md",
            "daily/2026/10/2026-09-21.md",
            "daily/2026/09",
            "projects/Not A Slug/project.toml",
            "projects/infra/notes/.deployment.md.0badf00d.tmp",
            "projects/infra/notes/deployment.txt",
            "projects/infra/notes/drafts/deployment.md",
            "tasks-archive-2026.toml",
            ".tasks.toml.0badf00d.tmp",
            ".git/index",
        ] {
            assert_eq!(change(ignored), None, "{ignored}");
        }
    }
}
