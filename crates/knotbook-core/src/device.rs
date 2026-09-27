//! Settings of this device, `.knotbook/device.toml`. The file is never
//! synced, so it holds what differs between machines, such as the paths of
//! the projects' local Git repositories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table};

use crate::error::{ReadError, SaveError};
use crate::file::{parse_text, read_optional};
use crate::toml_values::set;
use crate::{EditError, ProjectSlug, Vault};

impl Vault {
    /// Where the settings of this device live.
    pub(crate) fn device_path(&self) -> PathBuf {
        self.root().join(".knotbook").join("device.toml")
    }

    /// The paths of the projects' local Git repositories on this device.
    /// Entries of unknown projects are left out.
    pub fn repo_paths(&self) -> Result<BTreeMap<ProjectSlug, PathBuf>, ReadError> {
        let path = self.device_path();
        let Some(text) = read_optional(&path)? else {
            return Ok(BTreeMap::new());
        };
        let (_, mut repos) = parse_text(&path, &text, parse)?;
        repos.retain(|slug, _| self.project(slug).is_some());
        Ok(repos)
    }

    /// Sets the path of the local Git repository of the project `slug` on
    /// this device to `repo`, an absolute path, or removes it with `None`.
    /// Everything else in the file is kept as it is.
    pub fn set_repo_path(&self, slug: &ProjectSlug, repo: Option<&Path>) -> Result<(), SaveError> {
        if self.project(slug).is_none() {
            return Err(EditError::UnknownProject(slug.clone()).into());
        }
        let value = repo.map(repo_value).transpose()?;
        let path = self.device_path();
        let current = read_optional(&path)?;
        let mut document = match &current {
            Some(text) => parse_text(&path, text, parse)?.0,
            None => DocumentMut::new(),
        };
        let repos = document
            .entry("repos")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .expect("checked when parsing");
        match value {
            Some(value) => set(repos, slug.as_str(), value.into()),
            None => {
                repos.remove(slug.as_str());
            }
        }
        let text = document.to_string();
        if current.as_deref() != Some(text.as_str()) {
            self.write(&path, &text)?;
        }
        Ok(())
    }
}

/// Whether `repo` may be set as a project's repository: the absolute path
/// of a folder holding a Git repository. `.git` is a file in worktrees and
/// submodules.
pub fn check_repo_path(repo: &Path) -> Result<(), EditError> {
    repo_value(repo).map(|_| ())
}

/// `repo` as written to the file, if [`check_repo_path`] accepts it.
fn repo_value(repo: &Path) -> Result<&str, EditError> {
    let invalid = || EditError::InvalidRepoPath(repo.to_owned());
    let value = repo.to_str().ok_or_else(invalid)?;
    if !repo.is_absolute() {
        return Err(invalid());
    }
    if !repo.join(".git").exists() {
        return Err(EditError::NotARepository(repo.to_owned()));
    }
    Ok(value)
}

/// Reads the content `text` of `device.toml`: the document, to write it
/// back with comments and unknown entries, and the repository paths by slug.
/// Keys that are no slugs are left out.
fn parse(text: &str) -> Result<(DocumentMut, BTreeMap<ProjectSlug, PathBuf>), String> {
    let document: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| err.to_string())?;
    let Some(item) = document.get("repos") else {
        return Ok((document, BTreeMap::new()));
    };
    let table = item
        .as_table()
        .ok_or("repos must be a table, as in [repos]")?;
    let mut repos = BTreeMap::new();
    for (key, value) in table {
        let Ok(slug) = key.parse::<ProjectSlug>() else {
            continue;
        };
        let path = value
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| format!("the repository of {slug} must be an absolute path"))?;
        repos.insert(slug, path);
    }
    Ok((document, repos))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::file::{TempDir, sample_copy};

    fn slug(value: &str) -> ProjectSlug {
        value.parse().unwrap()
    }

    /// A folder that looks like a Git repository.
    fn repo(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.0.join(name);
        fs::create_dir_all(path.join(".git")).unwrap();
        path
    }

    #[test]
    fn no_file_means_no_repos() {
        let (_dir, vault) = sample_copy();
        assert!(vault.repo_paths().unwrap().is_empty());
    }

    #[test]
    fn set_and_remove() {
        let (_dir, vault) = sample_copy();
        let repos = TempDir::new();
        let infra = repo(&repos, "infra");
        vault.set_repo_path(&slug("infra"), Some(&infra)).unwrap();
        assert_eq!(
            fs::read_to_string(vault.device_path()).unwrap(),
            format!("[repos]\ninfra = \"{}\"\n", infra.display())
        );
        assert_eq!(
            vault.repo_paths().unwrap(),
            BTreeMap::from([(slug("infra"), infra)])
        );

        vault.set_repo_path(&slug("infra"), None).unwrap();
        assert!(vault.repo_paths().unwrap().is_empty());
        // Removing what is not there leaves the file alone.
        vault.set_repo_path(&slug("webshop"), None).unwrap();
    }

    #[test]
    fn writing_keeps_everything_else() {
        let (_dir, vault) = sample_copy();
        let repos = TempDir::new();
        let webshop = repo(&repos, "webshop");
        let text = "# This machine\n[repos]\nold-project = \"/code/old\"\ninfra = \"/code/infra\" # main checkout\n\n[editor]\ncommand = \"code\"\n";
        fs::create_dir_all(vault.device_path().parent().unwrap()).unwrap();
        fs::write(vault.device_path(), text).unwrap();
        assert_eq!(
            vault.repo_paths().unwrap(),
            BTreeMap::from([(slug("infra"), PathBuf::from("/code/infra"))])
        );

        vault
            .set_repo_path(&slug("webshop"), Some(&webshop))
            .unwrap();
        vault
            .set_repo_path(&slug("infra"), Some(&repo(&repos, "infra")))
            .unwrap();
        assert_eq!(
            fs::read_to_string(vault.device_path()).unwrap(),
            format!(
                "# This machine\n[repos]\nold-project = \"/code/old\"\ninfra = \"{0}/infra\" # main checkout\nwebshop = \"{0}/webshop\"\n\n[editor]\ncommand = \"code\"\n",
                repos.0.display()
            )
        );
    }

    #[test]
    fn only_absolute_repository_folders() {
        let (_dir, vault) = sample_copy();
        let repos = TempDir::new();
        let plain = repos.0.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let infra = slug("infra");
        assert_eq!(
            vault
                .set_repo_path(&infra, Some(&plain))
                .unwrap_err()
                .to_string(),
            format!("{} is not a Git repository", plain.display())
        );
        assert!(matches!(
            vault.set_repo_path(&infra, Some(Path::new("code/infra"))),
            Err(SaveError::Edit(EditError::InvalidRepoPath(_)))
        ));
        assert!(matches!(
            vault.set_repo_path(&slug("nope"), None),
            Err(SaveError::Edit(EditError::UnknownProject(_)))
        ));
        assert!(!vault.device_path().exists());
    }

    #[test]
    fn invalid_files_are_reported_and_kept() {
        let (_dir, vault) = sample_copy();
        fs::create_dir_all(vault.device_path().parent().unwrap()).unwrap();
        for text in [
            "repos = 1",
            "[repos]\ninfra = 1",
            "[repos]\ninfra = \"code/infra\"",
            "[repos",
        ] {
            fs::write(vault.device_path(), text).unwrap();
            let err = vault.repo_paths().unwrap_err().to_string();
            assert!(
                err.starts_with(&format!("invalid {}", vault.device_path().display())),
                "{err}"
            );
            assert!(vault.set_repo_path(&slug("infra"), None).is_err(), "{text}");
            assert_eq!(fs::read_to_string(vault.device_path()).unwrap(), text);
        }
    }
}
