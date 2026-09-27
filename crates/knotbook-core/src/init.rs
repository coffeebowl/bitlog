//! Creating new vaults.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use thiserror::Error;
use toml_edit::DocumentMut;

use crate::error::{ReadError, SaveError};
use crate::file::{read_folder, read_optional, write_atomic};
use crate::{Project, Vault};

const CONFIG: &str = include_str!("init/knotbook.toml");
const README: &str = include_str!("init/README.md");
const NOTE_TEMPLATE: &str = include_str!("init/note.md");

/// Lines that keep the local folder out of Git and Syncthing.
const IGNORE_ENTRIES: [(&str, &str); 2] =
    [(".gitignore", "/.knotbook/"), (".stignore", "/.knotbook")];

#[derive(Debug, Error)]
pub enum CreateError {
    #[error("{path} is not empty, a new vault needs a folder without visible files")]
    NotEmpty { path: PathBuf },
    #[error(transparent)]
    Save(#[from] SaveError),
}

impl From<ReadError> for CreateError {
    fn from(error: ReadError) -> Self {
        Self::Save(error.into())
    }
}

impl Vault {
    /// Creates a new vault named `name` in the folder `root` and opens it.
    ///
    /// The folder may be missing or hold hidden files only, such as `.git` or
    /// the marker folder of a sync tool. Existing `.gitignore` and
    /// `.stignore` files are extended, not replaced.
    pub fn create(root: &Path, name: &str, today: NaiveDate) -> Result<Self, CreateError> {
        check_empty(root)?;
        let mut config: DocumentMut = CONFIG.parse().expect("the default config is valid TOML");
        config["name"] = toml_edit::value(name);
        write_atomic(&root.join("knotbook.toml"), &config.to_string())?;
        write_atomic(&root.join("README.md"), README)?;
        write_atomic(&root.join("templates/note.md"), NOTE_TEMPLATE)?;
        for project in Project::defaults(today) {
            write_atomic(&Project::path(root, &project.slug), &project.to_toml())?;
        }
        for (file, entry) in IGNORE_ENTRIES {
            add_line(&root.join(file), entry)?;
        }
        Ok(Self::open(root)?)
    }
}

fn check_empty(root: &Path) -> Result<(), CreateError> {
    for entry in read_folder(root)? {
        if !entry.file_name().to_string_lossy().starts_with('.') {
            return Err(CreateError::NotEmpty {
                path: root.to_owned(),
            });
        }
    }
    Ok(())
}

/// Appends `line` to the file `path` unless it is there already.
fn add_line(path: &Path, line: &str) -> Result<(), SaveError> {
    let text = read_optional(path)?.unwrap_or_default();
    if text.lines().any(|existing| existing.trim() == line) {
        return Ok(());
    }
    let separator = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    write_atomic(path, &format!("{text}{separator}{line}\n"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::file::TempDir;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
    }

    fn sample_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault")
    }

    #[test]
    fn create_vault() {
        let dir = TempDir::new();
        let root = dir.0.join("new");
        let vault = Vault::create(&root, "Work \"log\"", today()).unwrap();

        let config = vault.config();
        assert_eq!(config.name, "Work \"log\"");
        assert_eq!(config.week.workdays.len(), 5);
        assert_eq!(config.grid.slot_minutes, 15);
        let locations: Vec<&str> = config.locations.keys().map(|key| key.as_str()).collect();
        assert_eq!(locations, ["hybrid", "office", "remote"]);
        assert_eq!(config.defaults.location, Some("remote".parse().unwrap()));
        // The comments stay for the user.
        let text = fs::read_to_string(root.join("knotbook.toml")).unwrap();
        assert!(
            text.contains("# partly remote, partly in the office"),
            "{text}"
        );

        let slugs: Vec<&str> = vault.projects().iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["filler", "meetings", "pause"]);
        assert_eq!(
            fs::read_to_string(root.join("templates/note.md")).unwrap(),
            fs::read_to_string(sample_path().join("templates/note.md")).unwrap()
        );
        assert!(root.join("README.md").is_file());
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/.knotbook/\n"
        );
        assert_eq!(
            fs::read_to_string(root.join(".stignore")).unwrap(),
            "/.knotbook\n"
        );
    }

    #[test]
    fn hidden_files_are_kept_and_extended() {
        let dir = TempDir::new();
        fs::create_dir_all(dir.0.join(".git")).unwrap();
        fs::write(dir.0.join(".gitignore"), "target").unwrap();
        fs::write(dir.0.join(".stignore"), "/.knotbook\n").unwrap();
        Vault::create(&dir.0, "Knotbook", today()).unwrap();
        assert!(dir.0.join(".git").is_dir());
        assert_eq!(
            fs::read_to_string(dir.0.join(".gitignore")).unwrap(),
            "target\n/.knotbook/\n"
        );
        assert_eq!(
            fs::read_to_string(dir.0.join(".stignore")).unwrap(),
            "/.knotbook\n"
        );
    }

    #[test]
    fn folder_with_visible_files_is_refused() {
        let err = Vault::create(&sample_path(), "Knotbook", today()).unwrap_err();
        assert!(matches!(err, CreateError::NotEmpty { .. }), "{err}");
    }
}
