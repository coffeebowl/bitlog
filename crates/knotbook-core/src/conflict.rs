//! Finding and merging the conflict copies that sync tools leave behind.
//!
//! A copy has no common ancestor with its original, so merging cannot tell
//! which side changed something. What both sides agree on or only one side
//! has is taken; everything else is a [`Contradiction`] left to the user.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{ReadError, SaveError};
use crate::file::{parse_text, read_optional, read_text};
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
    /// A block of the copy, the second, overlaps one of the original.
    Overlap(BlockId, BlockId),
    DayNote,
    /// A field of a task, by its name in the file.
    TaskField(TaskId, String),
    /// Notes, projects and the vault configuration are never merged, only
    /// kept as they are when both files are the same.
    Content,
}

impl fmt::Display for Contradiction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DayField(field) => write!(f, "the field {field} differs"),
            Self::Block(id) => write!(f, "block {id} differs"),
            Self::BlockText(id) => write!(f, "the text of block {id} differs"),
            Self::Overlap(ours, theirs) => write!(f, "blocks {ours} and {theirs} overlap"),
            Self::DayNote => write!(f, "the day note differs"),
            Self::TaskField(id, field) => write!(f, "the field {field} of task {id} differs"),
            Self::Content => write!(f, "the files differ"),
        }
    }
}

/// What merging a copy into its original gives, if nothing contradicts.
enum Merged {
    /// Both files are the same, or the original holds everything already.
    Kept,
    Day(DayFile, Day),
    Tasks(TaskList, TaskList),
    /// There is no original: the copy takes its place.
    Moved(String),
}

impl Vault {
    /// The conflict copies of the files Knotbook reads, sorted by path.
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
            // Leaves out .git, .knotbook and temporary files.
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().map_err(io_error)?.is_dir() {
                self.find_copies(&folder.join(&name), copies)?;
            } else if let Some(original) = original_name(&name)
                && let Some(of) = VaultChange::from_path(&folder.join(original))
            {
                copies.push(ConflictCopy {
                    path: folder.join(&name),
                    of,
                });
            }
        }
        Ok(())
    }

    /// What keeps `copy` from being merged into its original by
    /// [`Vault::merge_conflict`]; nothing if it can be.
    pub fn contradictions(&self, copy: &ConflictCopy) -> Result<Vec<Contradiction>, ReadError> {
        Ok(self.merged(copy)?.err().unwrap_or_default())
    }

    /// Merges `copy` into its original and removes it, unless something
    /// contradicts. Then nothing is changed and the contradictions are
    /// returned.
    pub fn merge_conflict(&self, copy: &ConflictCopy) -> Result<Vec<Contradiction>, SaveError> {
        match self.merged(copy)? {
            Err(contradictions) => return Ok(contradictions),
            Ok(Merged::Kept) => {}
            Ok(Merged::Day(file, day)) => {
                self.update_day(&file, |current| {
                    *current = day;
                    Ok(())
                })?;
            }
            Ok(Merged::Tasks(tasks, merged)) => {
                self.update_tasks(&tasks, |current| {
                    *current = merged;
                    Ok(())
                })?;
            }
            Ok(Merged::Moved(text)) => self.write(&self.original_path(copy), &text)?,
        }
        // Only now, so that nothing of the copy is lost if saving fails.
        let path = self.root().join(&copy.path);
        self.record_write(&path, None);
        match fs::remove_file(&path) {
            Ok(()) => Ok(Vec::new()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
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

    fn merged(&self, copy: &ConflictCopy) -> Result<Result<Merged, Vec<Contradiction>>, ReadError> {
        let copy_path = self.root().join(&copy.path);
        let theirs = read_text(&copy_path)?;
        let Some(ours) = read_optional(&self.original_path(copy))? else {
            return Ok(Ok(Merged::Moved(theirs)));
        };
        if ours == theirs {
            return Ok(Ok(Merged::Kept));
        }
        match &copy.of {
            VaultChange::Day(date) => {
                let file = self
                    .load_day(*date)?
                    .expect("the original was read just now");
                let other = parse_text(&copy_path, &theirs, |text| {
                    let (day, _) = Day::parse(text)?;
                    if day.date == *date {
                        Ok(day)
                    } else {
                        Err(format!("date {} does not match the original", day.date))
                    }
                })?;
                let mut day = file.day.clone();
                let contradictions = day.merge(&other);
                Ok(if contradictions.is_empty() {
                    Ok(Merged::Day(file, day))
                } else {
                    Err(contradictions)
                })
            }
            VaultChange::Tasks => {
                let tasks = self.load_tasks()?;
                let other = TaskList::read(&copy_path, &theirs)?;
                let mut merged = tasks.clone();
                let contradictions = merged.merge(&other, &self.archived_task_ids()?);
                Ok(if contradictions.is_empty() {
                    Ok(Merged::Tasks(tasks, merged))
                } else {
                    Err(contradictions)
                })
            }
            VaultChange::Config | VaultChange::Project(_) | VaultChange::Note(_) => {
                Ok(Err(vec![Contradiction::Content]))
            }
        }
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
            ("knotbook (conflicted copy).toml", "knotbook.toml"),
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
        fs::write(root.join("exports/week (conflicted copy).md"), "").ok();
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
        assert_eq!(vault.merge_conflict(&day_copy()).unwrap(), []);
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
        assert_eq!(vault.merge_conflict(&day_copy()).unwrap(), []);
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
        assert_eq!(vault.merge_conflict(&day_copy()).unwrap(), []);
        assert_eq!(fs::read_to_string(original).unwrap(), text);
    }

    #[test]
    fn notes_are_merged_only_when_equal() {
        let (_dir, vault) = sample_copy();
        let note = vault.root().join("projects/infra/notes/deployment.md");
        let copy = ConflictCopy {
            path: "projects/infra/notes/deployment (conflicted copy).md".into(),
            of: VaultChange::Note("projects/infra/notes/deployment.md".parse().unwrap()),
        };
        fs::write(vault.root().join(&copy.path), "Other text\n").unwrap();
        assert_eq!(
            vault.merge_conflict(&copy).unwrap(),
            [Contradiction::Content]
        );
        assert!(vault.root().join(&copy.path).exists());
        fs::copy(&note, vault.root().join(&copy.path)).unwrap();
        assert_eq!(vault.merge_conflict(&copy).unwrap(), []);
        assert!(!vault.root().join(&copy.path).exists());
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
        assert_eq!(vault.merge_conflict(&copy).unwrap(), []);
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
        assert_eq!(vault.merge_conflict(&copy).unwrap(), []);
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
        assert_eq!(
            vault.merge_conflict(&copy).unwrap(),
            [
                Contradiction::TaskField(id.clone(), "title".to_owned()),
                Contradiction::TaskField(id, "due".to_owned()),
            ]
        );
        assert!(vault.root().join(&copy.path).exists());
    }
}
