//! Finding and merging the conflict copies that sync tools leave behind.
//!
//! A copy has no common ancestor with its original, so merging cannot tell
//! which side changed something. What both sides agree on or only one side
//! has is taken; everything else is a [`Contradiction`] the user decides.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{ReadError, SaveError};
use crate::file::{parse_text, read_optional, read_text};
use crate::vault::EXPORTS;
use crate::{BlockId, Day, DayFile, TaskId, TaskList, Vault, VaultChange};

/// A conflict copy of a vault file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictCopy {
    /// Relative to the vault.
    pub path: PathBuf,
    /// The file it is a copy of.
    pub of: VaultChange,
}

/// Something a conflict copy and its original disagree on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contradiction {
    /// A field of the day's front matter, by its name in the file.
    DayField(String),
    /// The time, project or title of a block.
    Block(BlockId),
    BlockText(BlockId),
    /// A block only the copy has overlaps blocks of the original. Taking
    /// it removes them.
    Overlap(BlockId),
    DayNote,
    /// A field of a task, by its name in the file.
    TaskField(TaskId, String),
    /// Notes, projects and the vault configuration are never merged: one
    /// of the files is kept as a whole.
    Content,
}

impl fmt::Display for Contradiction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DayField(field) => write!(f, "the field {field} differs"),
            Self::Block(id) => write!(f, "block {id} differs"),
            Self::BlockText(id) => write!(f, "the text of block {id} differs"),
            Self::Overlap(id) => write!(f, "block {id} overlaps others"),
            Self::DayNote => write!(f, "the day note differs"),
            Self::TaskField(id, field) => write!(f, "the field {field} of task {id} differs"),
            Self::Content => write!(f, "the files differ"),
        }
    }
}

/// A conflict copy and its original, to compare them.
#[derive(Debug, Clone)]
pub enum ConflictVersions {
    Days(Day, Day),
    Tasks(TaskList, TaskList),
    /// The files as they are, the original empty if it is missing.
    Texts(String, String),
}

/// A copy and its original as read for merging.
enum Versions {
    /// There is no original, the text of the copy takes its place.
    Missing(String),
    Same(String),
    Days(DayFile, Day),
    Tasks(TaskList, TaskList),
    Texts(String, String),
}

/// What merging a copy into its original gives.
enum Merged {
    /// The original holds everything already.
    Kept,
    Day(DayFile, Day),
    Tasks(TaskList, TaskList),
    /// The text of the copy takes the original's place.
    Replaced(String),
}

/// The contradictions found while merging, and those decided for the copy.
pub(crate) struct Merger<'a> {
    theirs: &'a [Contradiction],
    pub found: Vec<Contradiction>,
}

impl<'a> Merger<'a> {
    pub fn new(theirs: &'a [Contradiction]) -> Self {
        Self {
            theirs,
            found: Vec::new(),
        }
    }

    /// Notes `contradiction` and returns whether the copy wins it.
    pub fn contradiction(&mut self, contradiction: Contradiction) -> bool {
        let wins = self.theirs.contains(&contradiction);
        self.found.push(contradiction);
        wins
    }

    /// Merges a value as [`take`] does. If both sides contradict, notes
    /// `contradiction` and takes `theirs` if the copy wins it.
    pub fn value<T: PartialEq + Clone>(
        &mut self,
        ours: &mut T,
        theirs: &T,
        empty: &T,
        contradiction: impl FnOnce() -> Contradiction,
    ) {
        if !take(ours, theirs, empty) && self.contradiction(contradiction()) {
            *ours = theirs.clone();
        }
    }
}

impl Vault {
    /// The conflict copies of the files BitLog reads, sorted by path.
    pub fn conflict_copies(&self) -> Result<Vec<ConflictCopy>, ReadError> {
        let mut copies = Vec::new();
        self.find_copies(Path::new(""), &mut copies)?;
        copies.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(copies)
    }

    fn find_copies(&self, folder: &Path, copies: &mut Vec<ConflictCopy>) -> Result<(), ReadError> {
        let path = self.root().join(folder);
        let io_error = |source| ReadError::Io {
            path: path.clone(),
            source,
        };
        for entry in fs::read_dir(&path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // Leaves out .git, .bitlog and temporary files.
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().map_err(io_error)?.is_dir() {
                // Exports are never read, and copies of assets are assets
                // of their own (see `copy_of`). Both folders can be large.
                let skipped = (folder.as_os_str().is_empty() && name == EXPORTS)
                    || (folder.parent() == Some(Path::new("projects")) && name == "assets");
                if !skipped {
                    self.find_copies(&folder.join(&name), copies)?;
                }
            } else if let Some(of) = copy_of(&folder.join(&name)) {
                copies.push(ConflictCopy {
                    path: folder.join(&name),
                    of,
                });
            }
        }
        Ok(())
    }

    /// What `copy` and its original disagree on, to be decided before
    /// [`Vault::merge_conflict`]; nothing if they can be merged as they are.
    pub fn contradictions(&self, copy: &ConflictCopy) -> Result<Vec<Contradiction>, ReadError> {
        Ok(self.merged(copy, &[])?.1)
    }

    /// `copy` and its original, to compare them.
    pub fn conflict_versions(&self, copy: &ConflictCopy) -> Result<ConflictVersions, ReadError> {
        Ok(match self.versions(copy)? {
            Versions::Missing(text) => ConflictVersions::Texts(String::new(), text),
            Versions::Same(text) => ConflictVersions::Texts(text.clone(), text),
            Versions::Days(file, theirs) => ConflictVersions::Days(file.day, theirs),
            Versions::Tasks(ours, theirs) => ConflictVersions::Tasks(ours, theirs),
            Versions::Texts(ours, theirs) => ConflictVersions::Texts(ours, theirs),
        })
    }

    /// Merges `copy` into its original and removes it. Of the
    /// [contradictions](Vault::contradictions), the copy wins those in
    /// `theirs` and the original all others.
    pub fn merge_conflict(
        &self,
        copy: &ConflictCopy,
        theirs: &[Contradiction],
    ) -> Result<(), SaveError> {
        match self.merged(copy, theirs)?.0 {
            Merged::Kept => {}
            Merged::Day(file, day) => {
                self.update_day(&file, |current| {
                    *current = day;
                    Ok(())
                })?;
            }
            Merged::Tasks(tasks, merged) => {
                self.update_tasks(&tasks, |current| {
                    *current = merged;
                    Ok(())
                })?;
            }
            Merged::Replaced(text) => self.write(&self.original_path(copy), &text)?,
        }
        // Only now, so that nothing of the copy is lost if saving fails.
        let path = self.root().join(&copy.path);
        self.record_write(&path, None);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(SaveError::Write { path, source }),
        }
    }

    fn original_path(&self, copy: &ConflictCopy) -> PathBuf {
        let name = copy
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(original_name)
            .expect("conflict copies are found by their names");
        self.root().join(copy.path.with_file_name(name))
    }

    fn versions(&self, copy: &ConflictCopy) -> Result<Versions, ReadError> {
        let copy_path = self.root().join(&copy.path);
        let theirs = read_text(&copy_path)?;
        let original = self.original_path(copy);
        let Some(ours) = read_optional(&original)? else {
            return Ok(Versions::Missing(theirs));
        };
        if ours == theirs {
            return Ok(Versions::Same(theirs));
        }
        // Parsed from the text read above, which may be gone from disk by now.
        Ok(match &copy.of {
            VaultChange::Day(date) => {
                let file = DayFile::read(&original, &ours)?;
                let other = parse_text(&copy_path, &theirs, |text| {
                    let (day, _) = Day::parse(text)?;
                    if day.date == *date {
                        Ok(day)
                    } else {
                        Err(format!("date {} does not match the original", day.date))
                    }
                })?;
                Versions::Days(file, other)
            }
            VaultChange::Tasks => Versions::Tasks(
                TaskList::read(&original, &ours)?,
                TaskList::read(&copy_path, &theirs)?,
            ),
            VaultChange::Config
            | VaultChange::Project(_)
            | VaultChange::Note(_)
            | VaultChange::Assets(_) => Versions::Texts(ours, theirs),
        })
    }

    /// The merge of `copy` into its original, where the copy wins the
    /// contradictions in `theirs`, and all contradictions found.
    fn merged(
        &self,
        copy: &ConflictCopy,
        theirs: &[Contradiction],
    ) -> Result<(Merged, Vec<Contradiction>), ReadError> {
        let mut merger = Merger::new(theirs);
        let merged = match self.versions(copy)? {
            Versions::Missing(text) => Merged::Replaced(text),
            Versions::Same(_) => Merged::Kept,
            Versions::Days(file, other) => {
                let mut day = file.day.clone();
                day.merge(&other, &mut merger);
                Merged::Day(file, day)
            }
            Versions::Tasks(tasks, other) => {
                let mut merged = tasks.clone();
                merged.merge(&other, &self.archived_task_ids()?, &mut merger);
                Merged::Tasks(tasks, merged)
            }
            Versions::Texts(_, text) => {
                if merger.contradiction(Contradiction::Content) {
                    Merged::Replaced(text)
                } else {
                    Merged::Kept
                }
            }
        };
        Ok((merged, merger.found))
    }

    /// The ids of all archived tasks.
    fn archived_task_ids(&self) -> Result<HashSet<TaskId>, ReadError> {
        let mut ids = HashSet::new();
        let entries = fs::read_dir(self.root()).map_err(|source| ReadError::Io {
            path: self.root().to_owned(),
            source,
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let is_archive = entry.file_name().to_str().is_some_and(|name| {
                name.starts_with("tasks-archive-")
                    && name.ends_with(".toml")
                    && original_name(name).is_none()
            });
            if is_archive {
                let archive = TaskList::read(&path, &read_text(&path)?)?;
                ids.extend(archive.tasks().iter().map(|task| task.id.clone()));
            }
        }
        Ok(ids)
    }
}

/// The name of the file that `name` is a conflict copy of, if it is one:
/// `a.sync-conflict-20260922-181530-KNOTBK7.md` (Syncthing) and
/// `a (conflicted copy 2026-09-22 181530).md` (Nextcloud) are copies of
/// `a.md`.
pub(crate) fn original_name(name: &str) -> Option<String> {
    if let Some((stem, rest)) = name.split_once(".sync-conflict-") {
        let extension = rest.find('.').map_or("", |dot| &rest[dot..]);
        return Some(format!("{stem}{extension}"));
    }
    let (stem, rest) = name.split_once(" (conflicted copy")?;
    let (_, extension) = rest.split_once(')')?;
    Some(format!("{stem}{extension}"))
}

/// The file that the file at `relative`, a path inside the vault, is a
/// conflict copy of, if it is one.
pub(crate) fn copy_of(relative: &Path) -> Option<VaultChange> {
    let original = original_name(relative.file_name()?.to_str()?)?;
    // Copies of assets are assets of their own.
    VaultChange::from_path(&relative.with_file_name(original))
        .filter(|change| !matches!(change, VaultChange::Assets(_)))
}

/// Makes `ours` the value both sides agree on, where `empty` on one side
/// gives way to the other. Returns false if they contradict.
pub(crate) fn take<T: PartialEq + Clone>(ours: &mut T, theirs: &T, empty: &T) -> bool {
    if *ours == *empty {
        *ours = theirs.clone();
        true
    } else {
        *theirs == *ours || *theirs == *empty
    }
}

/// Whether `text` holds the conflict markers Git leaves in a file when a
/// merge fails.
pub(crate) fn has_git_markers(text: &str) -> bool {
    let starts = |marker: &str| text.lines().any(|line| line.starts_with(marker));
    starts("<<<<<<<") && starts(">>>>>>>")
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::file::sample_copy;

    const COPY: &str = "daily/2026/09/2026-09-22.sync-conflict-20260922-181530-KNOTBK7.md";

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn day_copy() -> ConflictCopy {
        ConflictCopy {
            path: COPY.into(),
            of: VaultChange::Day(date(2026, 9, 22)),
        }
    }

    #[test]
    fn original_names() {
        for (copy, original) in [
            (
                "2026-09-22.sync-conflict-20260922-181530-KNOTBK7.md",
                "2026-09-22.md",
            ),
            (
                "tasks.sync-conflict-20260922-181530-KNOTBK7.toml",
                "tasks.toml",
            ),
            (
                "Deploy steps (conflicted copy 2026-09-22 181530).md",
                "Deploy steps.md",
            ),
            ("bitlog (conflicted copy).toml", "bitlog.toml"),
        ] {
            assert_eq!(original_name(copy).as_deref(), Some(original), "{copy}");
        }
        for name in ["2026-09-22.md", "tasks.toml", "sync-conflict notes.md"] {
            assert_eq!(original_name(name), None, "{name}");
        }
    }

    #[test]
    fn git_markers() {
        assert!(has_git_markers(
            "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> main\n"
        ));
        assert!(!has_git_markers("a\n=======\nb\n"));
    }

    #[test]
    fn find_copies() {
        let (_dir, vault) = sample_copy();
        let root = vault.root();
        fs::write(root.join("tasks (conflicted copy).toml"), "format = 1\n").unwrap();
        fs::write(
            root.join("projects/infra/notes/deployment.sync-conflict-20260922-181530-KNOTBK7.md"),
            "",
        )
        .unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/tasks (conflicted copy).toml"), "").unwrap();
        fs::create_dir_all(root.join("exports")).unwrap();
        fs::write(root.join("exports/week (conflicted copy).md"), "").unwrap();
        let assets = root.join("projects/infra/assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(
            assets.join("plan.sync-conflict-20260922-181530-KNOTBK7.pdf"),
            "",
        )
        .unwrap();
        let copies = vault.conflict_copies().unwrap();
        assert_eq!(
            copies,
            [
                day_copy(),
                ConflictCopy {
                    path:
                        "projects/infra/notes/deployment.sync-conflict-20260922-181530-KNOTBK7.md"
                            .into(),
                    of: VaultChange::Note("projects/infra/notes/deployment.md".parse().unwrap()),
                },
                ConflictCopy {
                    path: "tasks (conflicted copy).toml".into(),
                    of: VaultChange::Tasks,
                },
            ]
        );
        // A copy of a note is no note.
        assert_eq!(vault.notes(&"infra".parse().unwrap()).unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn find_copies_skips_exports_and_assets() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, vault) = sample_copy();
        let locked = [
            vault.root().join("exports/locked"),
            vault.root().join("projects/infra/assets/locked"),
        ];
        for folder in &locked {
            fs::create_dir_all(folder).unwrap();
            fs::set_permissions(folder, fs::Permissions::from_mode(0o000)).unwrap();
        }
        // Neither folder is entered, so what is in there does not matter.
        let copies = vault.conflict_copies();
        for folder in &locked {
            fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(copies.unwrap(), [day_copy()]);
    }

    #[test]
    fn merge_day_copy() {
        let (_dir, vault) = sample_copy();
        // The day note differs.
        assert_eq!(
            vault.contradictions(&day_copy()).unwrap(),
            [Contradiction::DayNote]
        );
        let copy = vault.root().join(COPY);
        let text = fs::read_to_string(&copy).unwrap();
        fs::write(&copy, text.replace(" Edited on the laptop.", "")).unwrap();
        // The copy is in another form, but holds the same.
        assert_eq!(vault.contradictions(&day_copy()).unwrap(), []);
        vault.merge_conflict(&day_copy(), &[]).unwrap();
        assert!(!copy.exists());
        let original = vault.day_path(date(2026, 9, 22));
        let before = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/sample-vault/daily/2026/09/2026-09-22.md"),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(original).unwrap(), before);
    }

    #[test]
    fn merge_new_blocks() {
        let (_dir, vault) = sample_copy();
        let copy = vault.root().join(COPY);
        let text = fs::read_to_string(&copy)
            .unwrap()
            .replace(" Edited on the laptop.", "")
            .replace(
                "blocks:\n",
                "blocks:\n  - id: zz11\n    start: 07:45\n    end: 08:45\n    project: infra\n",
            )
            .replace(
                "## Code review",
                "## Early fix {#zz11}\n\nRestarted the runner.\n\n## Code review",
            );
        fs::write(&copy, text).unwrap();
        assert_eq!(vault.contradictions(&day_copy()).unwrap(), []);
        vault.merge_conflict(&day_copy(), &[]).unwrap();
        let day = vault.load_day(date(2026, 9, 22)).unwrap().unwrap().day;
        let block = &day.blocks[0];
        assert_eq!(block.id.as_str(), "zz11");
        assert_eq!(block.title, "Early fix");
        assert_eq!(block.text, "Restarted the runner.");
        assert_eq!(day.blocks.len(), 6);
    }

    #[test]
    fn missing_original_is_replaced() {
        let (_dir, vault) = sample_copy();
        let original = vault.day_path(date(2026, 9, 22));
        let text = fs::read_to_string(vault.root().join(COPY)).unwrap();
        fs::remove_file(&original).unwrap();
        assert_eq!(vault.contradictions(&day_copy()).unwrap(), []);
        vault.merge_conflict(&day_copy(), &[]).unwrap();
        assert_eq!(fs::read_to_string(original).unwrap(), text);
    }

    #[test]
    fn notes_are_kept_or_replaced_as_a_whole() {
        let (_dir, vault) = sample_copy();
        let note = vault.root().join("projects/infra/notes/deployment.md");
        let original = fs::read_to_string(&note).unwrap();
        let copy = ConflictCopy {
            path: "projects/infra/notes/deployment (conflicted copy).md".into(),
            of: VaultChange::Note("projects/infra/notes/deployment.md".parse().unwrap()),
        };
        fs::write(vault.root().join(&copy.path), "Other text\n").unwrap();
        assert_eq!(
            vault.contradictions(&copy).unwrap(),
            [Contradiction::Content]
        );
        assert!(matches!(
            vault.conflict_versions(&copy).unwrap(),
            ConflictVersions::Texts(ours, theirs) if ours == original && theirs == "Other text\n"
        ));
        vault.merge_conflict(&copy, &[]).unwrap();
        assert_eq!(fs::read_to_string(&note).unwrap(), original);
        assert!(!vault.root().join(&copy.path).exists());

        fs::write(vault.root().join(&copy.path), "Other text\n").unwrap();
        vault
            .merge_conflict(&copy, &[Contradiction::Content])
            .unwrap();
        assert_eq!(fs::read_to_string(&note).unwrap(), "Other text\n");
    }

    #[test]
    fn merge_task_copy() {
        let (_dir, vault) = sample_copy();
        let tasks = vault.load_tasks().unwrap();
        let mut theirs = tasks.clone();
        theirs
            .set_due(&"t9x2".parse().unwrap(), Some(date(2026, 10, 2)))
            .unwrap();
        let added = theirs.add("Order new cables", date(2026, 9, 23)).unwrap();
        let copy = ConflictCopy {
            path: "tasks.sync-conflict-20260923-101010-KNOTBK7.toml".into(),
            of: VaultChange::Tasks,
        };
        fs::write(vault.root().join(&copy.path), theirs.to_toml()).unwrap();
        assert_eq!(vault.contradictions(&copy).unwrap(), []);
        vault.merge_conflict(&copy, &[]).unwrap();
        let merged = vault.load_tasks().unwrap();
        assert_eq!(
            merged.task(&"t9x2".parse().unwrap()).unwrap().due,
            Some(date(2026, 10, 2))
        );
        assert!(merged.task(&added).is_some());
        assert_eq!(merged.tasks().len(), 5);
    }

    fn task_copy(vault: &Vault, theirs: &TaskList) -> ConflictCopy {
        let copy = ConflictCopy {
            path: "tasks (conflicted copy).toml".into(),
            of: VaultChange::Tasks,
        };
        fs::write(vault.root().join(&copy.path), theirs.to_toml()).unwrap();
        copy
    }

    #[test]
    fn archived_tasks_stay_archived() {
        let (_dir, vault) = sample_copy();
        let tasks = vault.load_tasks().unwrap();
        // Archived here, while the copy still holds the finished tasks.
        vault.archive_tasks(&tasks, date(2026, 9, 23)).unwrap();
        let copy = task_copy(&vault, &tasks);
        assert_eq!(vault.contradictions(&copy).unwrap(), []);
        vault.merge_conflict(&copy, &[]).unwrap();
        assert_eq!(vault.load_tasks().unwrap().tasks().len(), 2);
    }

    #[test]
    fn different_task_fields_contradict() {
        let (_dir, vault) = sample_copy();
        let tasks = vault.load_tasks().unwrap();
        let id: TaskId = "h4c8".parse().unwrap();
        let mut theirs = tasks.clone();
        theirs.set_title(&id, "Renew all certificates").unwrap();
        theirs.set_due(&id, Some(date(2026, 10, 9))).unwrap();
        let copy = task_copy(&vault, &theirs);
        let title = Contradiction::TaskField(id.clone(), "title".to_owned());
        assert_eq!(
            vault.contradictions(&copy).unwrap(),
            [
                title.clone(),
                Contradiction::TaskField(id.clone(), "due".to_owned())
            ]
        );
        vault.merge_conflict(&copy, &[title]).unwrap();
        let merged = vault.load_tasks().unwrap();
        let task = merged.task(&id).unwrap();
        assert_eq!(task.title, "Renew all certificates");
        assert_eq!(task.due, Some(date(2026, 9, 30)));
    }
}
