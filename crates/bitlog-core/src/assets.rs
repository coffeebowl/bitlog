//! Project assets: files of any kind kept with a project, such as PDFs or
//! images, in folders of their own. BitLog lists, adds, moves and renames
//! them, but never reads them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{ReadError, SaveError};
use crate::file::{copy_atomic, read_folder};
use crate::project::project_folder;
use crate::{AssetPath, EditError, InvalidId, ProjectSlug, Vault};

/// A file in the assets folder of a project or one of its subfolders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub path: AssetPath,
    /// In bytes.
    pub size: u64,
    pub modified: SystemTime,
}

/// What a folder of the assets of a project holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetFolder {
    /// Sorted by name.
    pub folders: Vec<AssetPath>,
    /// Sorted by name.
    pub files: Vec<Asset>,
}

impl Vault {
    /// Where the assets of the project `project` live, a folder that may
    /// not exist yet.
    pub fn assets_folder(&self, project: &ProjectSlug) -> PathBuf {
        project_folder(self.root(), project).join("assets")
    }

    /// Where the asset `asset` lives.
    pub fn asset_path(&self, asset: &AssetPath) -> PathBuf {
        self.assets_folder(asset.project()).join(asset.path())
    }

    /// The asset of the project `project` at `path`, if it lies in its
    /// assets folder.
    pub fn asset_at(&self, project: &ProjectSlug, path: &Path) -> Option<AssetPath> {
        let relative = path.strip_prefix(self.assets_folder(project)).ok()?;
        let parts: Option<Vec<&str>> = relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect();
        AssetPath::new(project.clone(), &parts?.join("/")).ok()
    }

    /// Where `folder` lives, or the assets folder of `project`.
    pub fn asset_folder_path(&self, project: &ProjectSlug, folder: Option<&AssetPath>) -> PathBuf {
        folder.map_or_else(
            || self.assets_folder(project),
            |folder| self.asset_path(folder),
        )
    }

    /// What `folder` holds, or the top of the assets of the project
    /// `project`. Hidden files and folders are left out.
    pub fn asset_folder(
        &self,
        project: &ProjectSlug,
        folder: Option<&AssetPath>,
    ) -> Result<AssetFolder, ReadError> {
        let path = self.asset_folder_path(project, folder);
        let io_error = |source| ReadError::Io {
            path: path.clone(),
            source,
        };
        let mut found = AssetFolder::default();
        for entry in read_folder(&path)? {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(asset) = AssetPath::in_folder(project, folder, &name) else {
                continue;
            };
            // Links to folders are not followed, as they may lead in circles.
            if entry.file_type().map_err(io_error)?.is_dir() {
                found.folders.push(asset);
                continue;
            }
            // Leaves out broken links.
            let Ok(metadata) = fs::metadata(entry.path()) else {
                continue;
            };
            if metadata.is_file() {
                found.files.push(Asset {
                    path: asset,
                    size: metadata.len(),
                    modified: metadata.modified().map_err(io_error)?,
                });
            }
        }
        found.folders.sort();
        found.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }

    /// Copies the file or folder `source` into `folder`, or to the top of
    /// the assets of the project `project`. It keeps its name, with a number
    /// added if the name is taken, as in `plan (2).pdf`. A folder is copied
    /// with all it holds but hidden files and folders and links to folders.
    pub fn add_asset(
        &self,
        project: &ProjectSlug,
        folder: Option<&AssetPath>,
        source: &Path,
    ) -> Result<AssetPath, SaveError> {
        if self.project(project).is_none() {
            return Err(EditError::UnknownProject(project.clone()).into());
        }
        let metadata = fs::metadata(source).map_err(|err| read_error(source, err))?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(EditError::NotAFile(source.to_owned()).into());
        }
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let asset = self
            .free_asset(project, folder, name, metadata.is_dir())
            .map_err(EditError::from)?;
        if metadata.is_dir() {
            let into = self.asset_folder_path(project, folder);
            fs::create_dir_all(&into).map_err(|err| write_error(&into, err))?;
            let into = fs::canonicalize(&into).map_err(|err| read_error(&into, err))?;
            let source = fs::canonicalize(source).map_err(|err| read_error(source, err))?;
            // It would copy its own copy, again and again.
            if into.starts_with(&source) {
                return Err(EditError::IntoOwnFolder(source).into());
            }
        }
        self.copy_asset(source, metadata.is_dir(), &asset)?;
        Ok(asset)
    }

    /// Copies the file or folder `source` to `asset`, see [`Self::add_asset`].
    fn copy_asset(
        &self,
        source: &Path,
        is_folder: bool,
        asset: &AssetPath,
    ) -> Result<(), SaveError> {
        let path = self.asset_path(asset);
        if !is_folder {
            return copy_atomic(source, &path);
        }
        fs::create_dir_all(&path).map_err(|err| write_error(&path, err))?;
        for entry in read_folder(source)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let child = AssetPath::in_folder(asset.project(), Some(asset), &name)
                .map_err(EditError::from)?;
            let child_source = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| read_error(&child_source, err))?;
            if file_type.is_dir() {
                self.copy_asset(&child_source, true, &child)?;
            } else if fs::metadata(&child_source).is_ok_and(|metadata| metadata.is_file()) {
                self.copy_asset(&child_source, false, &child)?;
            }
        }
        Ok(())
    }

    /// Creates the folder `name` in `folder`, or at the top of the assets of
    /// the project `project`.
    pub fn create_asset_folder(
        &self,
        project: &ProjectSlug,
        folder: Option<&AssetPath>,
        name: &str,
    ) -> Result<AssetPath, SaveError> {
        if self.project(project).is_none() {
            return Err(EditError::UnknownProject(project.clone()).into());
        }
        let created = AssetPath::in_folder(project, folder, name).map_err(EditError::from)?;
        let path = self.asset_path(&created);
        if path.exists() {
            return Err(EditError::AssetExists(created).into());
        }
        fs::create_dir_all(&path).map_err(|err| write_error(&path, err))?;
        Ok(created)
    }

    /// Renames the file or folder `asset` to `name`, within its folder.
    pub fn rename_asset(&self, asset: &AssetPath, name: &str) -> Result<AssetPath, SaveError> {
        let renamed = asset.with_name(name).map_err(EditError::from)?;
        let from = self.asset_path(asset);
        let to = self.asset_path(&renamed);
        if !from.exists() {
            return Err(EditError::UnknownAsset(asset.clone()).into());
        }
        if to.exists() {
            return Err(EditError::AssetExists(renamed).into());
        }
        fs::rename(&from, &to).map_err(|err| write_error(&to, err))?;
        Ok(renamed)
    }

    /// Moves the file or folder `asset` into `folder`, or to the top of the
    /// assets of its project. It keeps its name, with a number added if the
    /// name is taken there, as in `plan (2).pdf`.
    pub fn move_asset(
        &self,
        asset: &AssetPath,
        folder: Option<&AssetPath>,
    ) -> Result<AssetPath, SaveError> {
        let from = self.asset_path(asset);
        if !from.exists() {
            return Err(EditError::UnknownAsset(asset.clone()).into());
        }
        if asset.parent().as_ref() == folder {
            return Ok(asset.clone());
        }
        if self
            .asset_folder_path(asset.project(), folder)
            .starts_with(&from)
        {
            return Err(EditError::IntoOwnFolder(from).into());
        }
        let moved = self
            .free_asset(asset.project(), folder, asset.name(), from.is_dir())
            .map_err(EditError::from)?;
        let to = self.asset_path(&moved);
        fs::rename(&from, &to).map_err(|err| write_error(&to, err))?;
        Ok(moved)
    }

    /// The asset `name` in `folder`, or at the top of the assets of the
    /// project `project`, with a number added if the name is taken: before
    /// the extension of a file, as in `plan (2).pdf`, after the name of a
    /// folder, as in `v1.2 (2)`.
    fn free_asset(
        &self,
        project: &ProjectSlug,
        folder: Option<&AssetPath>,
        name: &str,
        is_folder: bool,
    ) -> Result<AssetPath, InvalidId> {
        let mut asset = AssetPath::in_folder(project, folder, name)?;
        let mut number = 2;
        while self.asset_path(&asset).exists() {
            let name = if is_folder {
                format!("{name} ({number})")
            } else {
                numbered(name, number)
            };
            asset = AssetPath::in_folder(project, folder, &name)
                .expect("a number keeps the name valid");
            number += 1;
        }
        Ok(asset)
    }
}

fn read_error(path: &Path, source: std::io::Error) -> ReadError {
    ReadError::Io {
        path: path.to_owned(),
        source,
    }
}

fn write_error(path: &Path, source: std::io::Error) -> SaveError {
    SaveError::Write {
        path: path.to_owned(),
        source,
    }
}

/// `name` with `number` added before its extension, as in `plan (2).pdf`.
pub(crate) fn numbered(name: &str, number: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem} ({number}).{extension}"),
        None => format!("{name} ({number})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::{TempDir, sample_copy};

    fn slug(slug: &str) -> ProjectSlug {
        slug.parse().unwrap()
    }

    fn asset(path: &str) -> AssetPath {
        path.parse().unwrap()
    }

    fn edit_error(err: SaveError) -> EditError {
        match err {
            SaveError::Edit(err) => err,
            err => panic!("{err}"),
        }
    }

    /// Files at `paths` in the assets of `project`, a path ending in `/`
    /// an empty folder.
    fn put(vault: &Vault, project: &str, paths: &[&str]) {
        let folder = vault.assets_folder(&slug(project));
        for path in paths {
            let path = folder.join(path);
            if path.to_str().unwrap().ends_with('/') {
                fs::create_dir_all(path).unwrap();
            } else {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "12345").unwrap();
            }
        }
    }

    fn names(folder: &AssetFolder) -> (Vec<&str>, Vec<&str>) {
        (
            folder.folders.iter().map(AssetPath::name).collect(),
            folder.files.iter().map(|asset| asset.path.name()).collect(),
        )
    }

    #[test]
    fn list_folders() {
        let (_dir, vault) = sample_copy();
        let webshop = slug("webshop");
        assert_eq!(
            vault.asset_folder(&webshop, None).unwrap(),
            AssetFolder::default()
        );

        put(
            &vault,
            "webshop",
            &[
                "b.pdf",
                "a.pdf",
                "scans/2026/offer.png",
                "empty/",
                ".plan.pdf.0badf00d.tmp",
                ".cache/thumb.png",
            ],
        );
        let top = vault.asset_folder(&webshop, None).unwrap();
        assert_eq!(
            names(&top),
            (vec!["empty", "scans"], vec!["a.pdf", "b.pdf"])
        );
        assert_eq!(top.files[0].size, 5);
        let folder = vault.assets_folder(&webshop);
        assert_eq!(vault.asset_path(&top.files[0].path), folder.join("a.pdf"));
        let scans = vault.asset_folder(&webshop, Some(&top.folders[1])).unwrap();
        assert_eq!(names(&scans), (vec!["2026"], vec![]));
        let year = vault
            .asset_folder(&webshop, Some(&scans.folders[0]))
            .unwrap();
        assert_eq!(
            year.files[0].path,
            asset("projects/webshop/assets/scans/2026/offer.png")
        );
        // A folder removed elsewhere holds nothing.
        let gone = asset("projects/webshop/assets/gone");
        assert_eq!(
            vault.asset_folder(&webshop, Some(&gone)).unwrap(),
            AssetFolder::default()
        );
    }

    #[test]
    fn assets_at_paths() {
        let (_dir, vault) = sample_copy();
        let webshop = slug("webshop");
        let folder = vault.assets_folder(&webshop);
        assert_eq!(
            vault.asset_at(&webshop, &folder.join("scans/a.pdf")),
            Some(asset("projects/webshop/assets/scans/a.pdf"))
        );
        for elsewhere in [
            folder.clone(),
            folder.join(".cache/a.pdf"),
            vault.assets_folder(&slug("infra")).join("a.pdf"),
            vault.root().join("images/a.png"),
        ] {
            assert_eq!(vault.asset_at(&webshop, &elsewhere), None, "{elsewhere:?}");
        }
    }

    #[test]
    fn add_files() {
        let (_dir, vault) = sample_copy();
        let source = TempDir::new();
        fs::create_dir_all(&source.0).unwrap();
        let file = source.0.join("Offer 2026.pdf");
        fs::write(&file, "offer").unwrap();

        let added = vault.add_asset(&slug("webshop"), None, &file).unwrap();
        assert_eq!(added, asset("projects/webshop/assets/Offer 2026.pdf"));
        assert_eq!(
            fs::read_to_string(vault.asset_path(&added)).unwrap(),
            "offer"
        );
        let again = vault.add_asset(&slug("webshop"), None, &file).unwrap();
        assert_eq!(again, asset("projects/webshop/assets/Offer 2026 (2).pdf"));
        let third = vault.add_asset(&slug("webshop"), None, &file).unwrap();
        assert_eq!(third, asset("projects/webshop/assets/Offer 2026 (3).pdf"));
        // No temporary files are left behind.
        assert_eq!(
            fs::read_dir(vault.assets_folder(&slug("webshop")))
                .unwrap()
                .count(),
            3
        );

        let scans = vault
            .create_asset_folder(&slug("webshop"), None, "scans")
            .unwrap();
        let inside = vault
            .add_asset(&slug("webshop"), Some(&scans), &file)
            .unwrap();
        assert_eq!(
            inside,
            asset("projects/webshop/assets/scans/Offer 2026.pdf")
        );

        assert_eq!(
            edit_error(vault.add_asset(&slug("nothing"), None, &file).unwrap_err()),
            EditError::UnknownProject(slug("nothing"))
        );
        let missing = source.0.join("missing.pdf");
        assert!(matches!(
            vault.add_asset(&slug("webshop"), None, &missing),
            Err(SaveError::Read(ReadError::Io { .. }))
        ));
    }

    #[test]
    fn add_folders() {
        let (_dir, vault) = sample_copy();
        let source = TempDir::new();
        let folder = source.0.join("v1.2");
        for path in ["a.pdf", "scans/b.png", ".git/config", "empty/x"] {
            let path = folder.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        fs::remove_file(folder.join("empty/x")).unwrap();

        let added = vault.add_asset(&slug("webshop"), None, &folder).unwrap();
        assert_eq!(added, asset("projects/webshop/assets/v1.2"));
        let mut copied: Vec<_> = walk(&vault.asset_path(&added));
        copied.sort();
        assert_eq!(copied, ["a.pdf", "empty", "scans", "scans/b.png"]);
        let again = vault.add_asset(&slug("webshop"), None, &folder).unwrap();
        assert_eq!(again, asset("projects/webshop/assets/v1.2 (2)"));

        // Nor into a folder of its own, nor into the vault it lies in.
        let assets = vault.assets_folder(&slug("webshop"));
        assert_eq!(
            edit_error(
                vault
                    .add_asset(&slug("webshop"), Some(&added), &assets)
                    .unwrap_err()
            ),
            EditError::IntoOwnFolder(fs::canonicalize(&assets).unwrap())
        );
        assert!(matches!(
            edit_error(
                vault
                    .add_asset(&slug("infra"), None, vault.root())
                    .unwrap_err()
            ),
            EditError::IntoOwnFolder(_)
        ));
    }

    /// The paths of the files and folders in `folder`, relative to it.
    fn walk(folder: &Path) -> Vec<String> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if entry.file_type().unwrap().is_dir() {
                paths.extend(
                    walk(&entry.path())
                        .into_iter()
                        .map(|path| format!("{name}/{path}")),
                );
            }
            paths.push(name);
        }
        paths
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("plan.pdf", 2), "plan (2).pdf");
        assert_eq!(numbered("backup.tar.gz", 3), "backup.tar (3).gz");
        assert_eq!(numbered("Makefile", 2), "Makefile (2)");
    }

    #[test]
    fn create_folders() {
        let (_dir, vault) = sample_copy();
        let infra = slug("infra");
        let scans = vault.create_asset_folder(&infra, None, "scans").unwrap();
        let year = vault
            .create_asset_folder(&infra, Some(&scans), "2026")
            .unwrap();
        assert_eq!(year, asset("projects/infra/assets/scans/2026"));
        assert!(vault.asset_path(&year).is_dir());

        assert_eq!(
            edit_error(
                vault
                    .create_asset_folder(&infra, None, "scans")
                    .unwrap_err()
            ),
            EditError::AssetExists(scans)
        );
        for invalid in ["", ".cache", "a/b"] {
            assert!(matches!(
                edit_error(
                    vault
                        .create_asset_folder(&infra, None, invalid)
                        .unwrap_err()
                ),
                EditError::InvalidId(_)
            ));
        }
        assert_eq!(
            edit_error(
                vault
                    .create_asset_folder(&slug("nothing"), None, "a")
                    .unwrap_err()
            ),
            EditError::UnknownProject(slug("nothing"))
        );
    }

    #[test]
    fn rename_assets() {
        let (_dir, vault) = sample_copy();
        put(&vault, "infra", &["scans/a.pdf", "scans/b.pdf"]);
        let folder = vault.assets_folder(&slug("infra"));
        let a = asset("projects/infra/assets/scans/a.pdf");

        let renamed = vault.rename_asset(&a, "c.pdf").unwrap();
        assert_eq!(renamed, asset("projects/infra/assets/scans/c.pdf"));
        assert!(folder.join("scans/c.pdf").is_file());
        assert!(!folder.join("scans/a.pdf").exists());

        let scans = asset("projects/infra/assets/scans");
        let docs = vault.rename_asset(&scans, "docs").unwrap();
        assert_eq!(docs, asset("projects/infra/assets/docs"));
        assert!(folder.join("docs/c.pdf").is_file());

        assert_eq!(
            edit_error(vault.rename_asset(&a, "d.pdf").unwrap_err()),
            EditError::UnknownAsset(a)
        );
        let c = asset("projects/infra/assets/docs/c.pdf");
        assert_eq!(
            edit_error(vault.rename_asset(&c, "b.pdf").unwrap_err()),
            EditError::AssetExists(asset("projects/infra/assets/docs/b.pdf"))
        );
        assert!(matches!(
            edit_error(vault.rename_asset(&c, "../c.pdf").unwrap_err()),
            EditError::InvalidId(_)
        ));
    }

    #[test]
    fn move_assets() {
        let (_dir, vault) = sample_copy();
        put(
            &vault,
            "infra",
            &["a.pdf", "old/a.pdf", "scans/2026/", "scans/x.png"],
        );
        let folder = vault.assets_folder(&slug("infra"));
        let scans = asset("projects/infra/assets/scans");
        let year = asset("projects/infra/assets/scans/2026");

        let a = asset("projects/infra/assets/a.pdf");
        let moved = vault.move_asset(&a, Some(&year)).unwrap();
        assert_eq!(moved, asset("projects/infra/assets/scans/2026/a.pdf"));
        assert!(folder.join("scans/2026/a.pdf").is_file());
        assert!(!folder.join("a.pdf").exists());

        // Up again, and a name taken there gets a number.
        let up = vault.move_asset(&moved, None).unwrap();
        assert_eq!(up, asset("projects/infra/assets/a.pdf"));
        let old = asset("projects/infra/assets/old/a.pdf");
        assert_eq!(
            vault.move_asset(&old, None).unwrap(),
            asset("projects/infra/assets/a (2).pdf")
        );
        // Staying in its folder changes nothing.
        assert_eq!(vault.move_asset(&up, None).unwrap(), up);
        assert!(folder.join("a.pdf").is_file());

        let old = asset("projects/infra/assets/old");
        let into_scans = vault.move_asset(&old, Some(&scans)).unwrap();
        assert_eq!(into_scans, asset("projects/infra/assets/scans/old"));
        assert!(walk(&folder.join("scans/old")).is_empty());

        for into in [&scans, &year] {
            assert_eq!(
                edit_error(vault.move_asset(&scans, Some(into)).unwrap_err()),
                EditError::IntoOwnFolder(folder.join("scans"))
            );
        }
        assert_eq!(
            edit_error(vault.move_asset(&old, None).unwrap_err()),
            EditError::UnknownAsset(old)
        );
    }
}
