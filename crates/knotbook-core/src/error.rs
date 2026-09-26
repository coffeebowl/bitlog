use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::EditError;

/// A vault file that cannot be read or does not follow the format.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid {path}: {message}")]
    Invalid { path: PathBuf, message: String },
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
