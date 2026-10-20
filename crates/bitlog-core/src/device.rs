//! Settings of this device, `.bitlog/device.toml`. The file is never
//! synced, so it holds what differs between machines, such as the paths of
//! the projects' local Git repositories.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table};

use crate::error::{ReadError, SaveError};
use crate::file::{parse_text, read_optional};
use crate::toml_values::set;
use crate::{EditError, ProjectSlug, Vault};

impl Vault {
    /// Where the settings of this device live.
    pub(crate) fn device_path(&self) -> PathBuf {
        self.root().join(".bitlog").join("device.toml")
    }

    /// The paths of the projects' local Git repositories on this device, in
    /// the order of the file. Entries of unknown projects and projects
    /// without repositories are left out.
    pub fn repo_paths(&self) -> Result<BTreeMap<ProjectSlug, Vec<PathBuf>>, ReadError> {
        let path = self.device_path();
        let Some(text) = read_optional(&path)? else {
            return Ok(BTreeMap::new());
        };
        let (_, mut repos) = parse_text(&path, &text, parse)?;
        repos.retain(|slug, paths| self.project(slug).is_some() && !paths.is_empty());
        Ok(repos)
    }

    /// Sets the paths of the local Git repositories of the project `slug`
    /// on this device to `repos`, absolute paths; none removes the entry.
    /// Only new paths are checked: those set before may have moved away
    /// since and stay until removed. Everything else in the file is kept as
    /// it is.
    pub fn set_repo_paths(&self, slug: &ProjectSlug, repos: &[PathBuf]) -> Result<(), SaveError> {
        if self.project(slug).is_none() {
            return Err(EditError::UnknownProject(slug.clone()).into());
        }
        let path = self.device_path();
        let current = read_optional(&path)?;
        let (mut document, old) = match &current {
            Some(text) => parse_text(&path, text, parse)?,
            None => (DocumentMut::new(), BTreeMap::new()),
        };
        let kept = old.get(slug).map(Vec::as_slice).unwrap_or_default();
        let values: Array = repos
            .iter()
            .map(|repo| match kept.contains(repo) {
                true => Ok(repo.to_str().expect("read from the file as text")),
                false => repo_value(repo),
            })
            .collect::<Result<_, _>>()?;
        let table = document
            .entry("repos")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .expect("checked when parsing");
        if repos.is_empty() {
            table.remove(slug.as_str());
        } else if old.get(slug).map(Vec::as_slice) != Some(repos) {
            set(table, slug.as_str(), values.into());
        }
        let text = document.to_string();
        if current.as_deref() != Some(text.as_str()) {
            self.write(&path, &text)?;
        }
        Ok(())
    }

    /// Moves the repositories of the project `from` on this device to the
    /// project `to`, as when the project is renamed.
    pub(crate) fn rename_repo_paths(
        &self,
        from: &ProjectSlug,
        to: &ProjectSlug,
    ) -> Result<(), SaveError> {
        let path = self.device_path();
        let Some(text) = read_optional(&path)? else {
            return Ok(());
        };
        let mut document = parse_text(&path, &text, parse)?.0;
        let Some(repos) = document.get_mut("repos").and_then(Item::as_table_mut) else {
            return Ok(());
        };
        if let Some(repo) = repos.remove(from.as_str()) {
            repos.insert(to.as_str(), repo);
            self.write(&path, &document.to_string())?;
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

/// The name of the repository in the folder `repo`, the folder's name.
pub fn repo_name(repo: &Path) -> String {
    repo.file_name().map_or_else(
        || repo.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Reads the content `text` of `device.toml`: the document, to write it
/// back with comments and unknown entries, and the repository paths by slug.
/// Keys that are no slugs are left out.
fn parse(text: &str) -> Result<(DocumentMut, BTreeMap<ProjectSlug, Vec<PathBuf>>), String> {
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
        let paths = value
            .as_array()
            .and_then(|paths| {
                paths
                    .iter()
                    .map(|path| {
                        let path = PathBuf::from(path.as_str()?);
                        path.is_absolute().then_some(path)
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| {
                format!(
                    "the repositories of {slug} must be a list of absolute paths, \
                     as in [\"/home/me/code/project\"]"
                )
            })?;
        repos.insert(slug, paths);
    }
    Ok((document, repos))
}

#[cfg(test)]
mod tests {
    use std::{fs, slice};

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
        let proto = repo(&repos, "proto");
        let both = [infra.clone(), proto.clone()];
        vault.set_repo_paths(&slug("infra"), &both).unwrap();
        assert_eq!(
            fs::read_to_string(vault.device_path()).unwrap(),
            format!(
                "[repos]\ninfra = [\"{}\", \"{}\"]\n",
                infra.display(),
                proto.display()
            )
        );
        assert_eq!(
            vault.repo_paths().unwrap(),
            BTreeMap::from([(slug("infra"), both.to_vec())])
        );

        vault.set_repo_paths(&slug("infra"), &[]).unwrap();
        assert!(vault.repo_paths().unwrap().is_empty());
        // Removing what is not there leaves the file alone.
        vault.set_repo_paths(&slug("webshop"), &[]).unwrap();
    }

    #[test]
    fn writing_keeps_everything_else() {
        let (_dir, vault) = sample_copy();
        let repos = TempDir::new();
        let webshop = repo(&repos, "webshop");
        let text = "# This machine\n[repos]\nold-project = [\"/code/old\"]\ninfra = [\"/code/infra\"] # main checkout\nmeetings = []\n\n[editor]\ncommand = \"code\"\n";
        fs::create_dir_all(vault.device_path().parent().unwrap()).unwrap();
        fs::write(vault.device_path(), text).unwrap();
        assert_eq!(
            vault.repo_paths().unwrap(),
            BTreeMap::from([(slug("infra"), vec![PathBuf::from("/code/infra")])])
        );

        vault.set_repo_paths(&slug("webshop"), &[webshop]).unwrap();
        vault
            .set_repo_paths(&slug("infra"), &[repo(&repos, "infra")])
            .unwrap();
        assert_eq!(
            fs::read_to_string(vault.device_path()).unwrap(),
            format!(
                "# This machine\n[repos]\nold-project = [\"/code/old\"]\ninfra = [\"{0}/infra\"] # main checkout\nmeetings = []\nwebshop = [\"{0}/webshop\"]\n\n[editor]\ncommand = \"code\"\n",
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
                .set_repo_paths(&infra, slice::from_ref(&plain))
                .unwrap_err()
                .to_string(),
            format!("{} is not a Git repository", plain.display())
        );
        assert!(matches!(
            vault.set_repo_paths(&infra, &["code/infra".into()]),
            Err(SaveError::Edit(EditError::InvalidRepoPath(_)))
        ));
        assert!(matches!(
            vault.set_repo_paths(&slug("nope"), &[]),
            Err(SaveError::Edit(EditError::UnknownProject(_)))
        ));
        assert!(!vault.device_path().exists());
    }

    #[test]
    fn repositories_moved_away_stay() {
        let (_dir, vault) = sample_copy();
        let repos = TempDir::new();
        let (api, proto) = (repo(&repos, "api"), repo(&repos, "proto"));
        let infra = slug("infra");
        vault
            .set_repo_paths(&infra, &[api.clone(), proto.clone()])
            .unwrap();
        fs::remove_dir_all(&api).unwrap();
        fs::remove_dir_all(&proto).unwrap();
        vault.set_repo_paths(&infra, slice::from_ref(&api)).unwrap();
        assert_eq!(vault.repo_paths().unwrap()[&infra], slice::from_ref(&api));
        // Only new paths are checked.
        assert!(vault.set_repo_paths(&infra, &[api, proto]).is_err());
    }

    #[test]
    fn invalid_files_are_reported_and_kept() {
        let (_dir, vault) = sample_copy();
        fs::create_dir_all(vault.device_path().parent().unwrap()).unwrap();
        for text in [
            "repos = 1",
            "[repos]\ninfra = 1",
            "[repos]\ninfra = \"/code/infra\"",
            "[repos]\ninfra = [\"/code/infra\", \"code/proto\"]",
            "[repos",
        ] {
            fs::write(vault.device_path(), text).unwrap();
            let err = vault.repo_paths().unwrap_err().to_string();
            assert!(
                err.starts_with(&format!("invalid {}", vault.device_path().display())),
                "{err}"
            );
            assert!(vault.set_repo_paths(&slug("infra"), &[]).is_err(), "{text}");
            assert_eq!(fs::read_to_string(vault.device_path()).unwrap(), text);
        }
    }
}
