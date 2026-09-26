use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// A vault file that cannot be read or does not follow the format.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid {path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

/// Reads `path` and parses it, attaching the path to any error.
pub(crate) fn read_file<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, ReadError> {
    let text = fs::read_to_string(path).map_err(|source| ReadError::Io {
        path: path.to_owned(),
        source,
    })?;
    parse(&text).map_err(|message| ReadError::Invalid {
        path: path.to_owned(),
        message,
    })
}
