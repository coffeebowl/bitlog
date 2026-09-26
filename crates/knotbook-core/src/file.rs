//! Reading and safely writing vault files.

use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::path::Path;

use crate::conflict::has_git_markers;
use crate::error::{ReadError, SaveError};

/// Reads `path` and parses it, attaching the path to any error.
pub(crate) fn read_file<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, ReadError> {
    let text = read_text(path)?;
    parse_text(path, &text, parse)
}

pub(crate) fn read_text(path: &Path) -> Result<String, ReadError> {
    fs::read_to_string(path).map_err(|source| ReadError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Reads `path`, or returns `None` if there is no such file.
pub(crate) fn read_optional(path: &Path) -> Result<Option<String>, ReadError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ReadError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Parses `text` read from `path`, attaching the path to any error.
pub(crate) fn parse_text<T>(
    path: &Path,
    text: &str,
    parse: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, ReadError> {
    parse(text).map_err(|message| ReadError::Invalid {
        path: path.to_owned(),
        message: if has_git_markers(text) {
            "it holds Git conflict markers, resolve them with Git".to_owned()
        } else {
            message
        },
    })
}

/// Identifies the content of a file, to notice changes made elsewhere.
///
/// The standard hasher is not stable across Rust versions. That does not
/// matter: hashes are compared while the program runs, and a hash kept in the
/// index only makes it read a file once more after an update.
pub fn content_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Writes `text` to `path` so that readers, sync tools included, see either
/// the old or the new content, never a part of it. Creates missing folders.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), SaveError> {
    let folder = path.parent().expect("vault files lie in a folder");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("vault file names are UTF-8");
    // In the same folder, so that renaming never crosses file systems.
    let temporary = folder.join(format!(".{name}.{:08x}.tmp", fastrand::u32(..)));
    let result = fs::create_dir_all(folder)
        .and_then(|()| {
            let mut file = File::create_new(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    result.map_err(|source| {
        // Nothing to do if it was never created.
        let _ = fs::remove_file(&temporary);
        SaveError::Write {
            path: path.to_owned(),
            source,
        }
    })
}

/// A fresh folder for a test, removed again when dropped.
#[cfg(test)]
pub(crate) struct TempDir(pub std::path::PathBuf);

#[cfg(test)]
impl TempDir {
    pub fn new() -> Self {
        Self(std::env::temp_dir().join(format!("knotbook-test-{:016x}", fastrand::u64(..))))
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A copy of the sample vault that a test may change, removed again when
/// the returned folder is dropped.
#[cfg(test)]
pub(crate) fn sample_copy() -> (TempDir, crate::Vault) {
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
    let vault = crate::Vault::open(&dir.0).unwrap();
    (dir, vault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_folders_and_replaces_files() {
        let dir = TempDir::new();
        let path = dir.0.join("daily/2026/10/2026-10-01.md");
        write_atomic(&path, "first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");
        write_atomic(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        // No temporary files are left behind.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_names_file() {
        let dir = TempDir::new();
        // A file where the folder should be.
        write_atomic(&dir.0.join("daily"), "").unwrap();
        let path = dir.0.join("daily/2026-10-01.md");
        let err = write_atomic(&path, "text").unwrap_err().to_string();
        assert!(
            err.starts_with(&format!("cannot write {}", path.display())),
            "{err}"
        );
    }

    #[test]
    fn missing_file_is_none() {
        let dir = TempDir::new();
        assert!(read_optional(&dir.0.join("none.md")).unwrap().is_none());
    }

    #[test]
    fn hash_tells_contents_apart() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("a\n"));
    }
}
