use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::{BlockId, LocationKey, ProjectSlug, TaskId};

/// A vault file that cannot be read or does not follow the format.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid {path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

/// A change that would make a day, project or task list invalid.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EditError {
    #[error("there is no block {0}")]
    UnknownBlock(BlockId),
    #[error("there is no project {0}")]
    UnknownProject(ProjectSlug),
    #[error("there is no location {0}")]
    UnknownLocation(LocationKey),
    #[error("a block cannot start and end at the same time")]
    EmptyBlock,
    #[error("the block would overlap block {0}")]
    Overlap(BlockId),
    #[error("a block title has to fit on one line")]
    MultilineTitle,
    #[error("the project {0} exists already")]
    ProjectExists(ProjectSlug),
    #[error("a color has to look like #3584e4, found {0:?}")]
    InvalidColor(String),
    #[error("unknown project status {0:?}, expected active, paused or archived")]
    InvalidStatus(String),
    #[error("there is no task {0}")]
    UnknownTask(TaskId),
    #[error("a task needs a title of one line")]
    InvalidTaskTitle,
}

/// A vault file that cannot be saved. Saving reads the file again first if
/// it was changed elsewhere, which can fail as well, and the change may not
/// fit the file as it is now.
#[derive(Debug, Error)]
pub enum SaveError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}
