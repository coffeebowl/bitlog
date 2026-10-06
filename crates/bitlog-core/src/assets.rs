//! Project assets: files of any kind kept with a project, such as PDFs or
//! images. BitLog lists, adds and renames them, but never reads them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{ReadError, SaveError};
use crate::file::{copy_atomic, read_folder};
use crate::project::project_folder;
use crate::{AssetPath, EditError, ProjectSlug, Vault};

/// A file in the assets folder of a project or one of its subfolders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub path: AssetPath,
    /// In bytes.
    pub size: u64,
    pub modified: SystemTime,
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

    /// The assets of the project `project`, those in subfolders included,
    /// sorted by path. Hidden files and folders are left out.
    pub fn assets(&self, project: &ProjectSlug) -> Result<Vec<Asset>, ReadError> {
        let mut assets = Vec::new();
        self.find_assets(project, "", &mut assets)?;
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(assets)
    }

    fn find_assets(
        &self,
        project: &ProjectSlug,
        folder: &str,
        assets: &mut Vec<Asset>,
    ) -> Result<(), ReadError> {
        let path = self.assets_folder(project).join(folder);
        let io_error = |source| ReadError::Io {
            path: path.clone(),
            source,
        };
        for entry in read_folder(&path)? {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let relative = if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            };
            let Ok(asset) = AssetPath::new(project.clone(), &relative) else {
                continue;
            };
            // Links to folders are not followed, as they may lead in circles.
            if entry.file_type().map_err(io_error)?.is_dir() {
                self.find_assets(project, &relative, assets)?;
                continue;
            }
            // Leaves out broken links.
            let Ok(metadata) = fs::metadata(entry.path()) else {
                continue;
            };
            if metadata.is_file() {
                assets.push(Asset {
                    path: asset,
                    size: metadata.len(),
                    modified: metadata.modified().map_err(io_error)?,
                });
            }
        }
        Ok(())
    }

    /// Copies the file `source` into the assets folder of the project
    /// `project`. It keeps its name, with a number added if the name is
    /// taken, as in `plan (2).pdf`.
    pub fn add_asset(&self, project: &ProjectSlug, source: &Path) -> Result<AssetPath, SaveError> {
        if self.project(project).is_none() {
            return Err(EditError::UnknownProject(project.clone()).into());
        }
        let metadata = fs::metadata(source).map_err(|source_error| ReadError::Io {
            path: source.to_owned(),
            source: source_error,
        })?;
        if !metadata.is_file() {
            return Err(EditError::NotAFile(source.to_owned()).into());
        }
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let mut asset = AssetPath::new(project.clone(), name).map_err(EditError::from)?;
        let mut number = 2;
        while self.asset_path(&asset).exists() {
            asset = AssetPath::new(project.clone(), &numbered(name, number))
                .expect("a number keeps the name valid");
            number += 1;
        }
        copy_atomic(source, &self.asset_path(&asset))?;
        Ok(asset)
    }

    /// Renames the asset `asset` to `name`, within its folder.
    pub fn rename_asset(&self, asset: &AssetPath, name: &str) -> Result<AssetPath, SaveError> {
        let renamed = asset.with_name(name).map_err(EditError::from)?;
        let from = self.asset_path(asset);
        let to = self.asset_path(&renamed);
        if !from.is_file() {
            return Err(EditError::UnknownAsset(asset.clone()).into());
        }
        if to.exists() {
            return Err(EditError::AssetExists(renamed).into());
        }
        fs::rename(&from, &to).map_err(|source| SaveError::Write { path: to, source })?;
        Ok(renamed)
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

    #[test]
    fn list_assets() {
        let (_dir, vault) = sample_copy();
        assert!(vault.assets(&slug("webshop")).unwrap().is_empty());

        let folder = vault.assets_folder(&slug("webshop"));
        fs::create_dir_all(folder.join("scans/2026")).unwrap();
        fs::create_dir_all(folder.join(".cache")).unwrap();
        fs::write(folder.join("plan.pdf"), "12345").unwrap();
        fs::write(folder.join("scans/2026/offer.png"), "").unwrap();
        fs::write(folder.join(".plan.pdf.0badf00d.tmp"), "").unwrap();
        fs::write(folder.join(".cache/thumb.png"), "").unwrap();
        let assets = vault.assets(&slug("webshop")).unwrap();
        let paths: Vec<_> = assets.iter().map(|asset| asset.path.to_string()).collect();
        assert_eq!(
            paths,
            [
                "projects/webshop/assets/plan.pdf",
                "projects/webshop/assets/scans/2026/offer.png"
            ]
        );
        assert_eq!(assets[0].size, 5);
        assert_eq!(vault.asset_path(&assets[0].path), folder.join("plan.pdf"));
    }

    #[test]
    fn add_assets() {
        let (_dir, vault) = sample_copy();
        let source = TempDir::new();
        fs::create_dir_all(&source.0).unwrap();
        let file = source.0.join("Offer 2026.pdf");
        fs::write(&file, "offer").unwrap();

        let added = vault.add_asset(&slug("webshop"), &file).unwrap();
        assert_eq!(added, asset("projects/webshop/assets/Offer 2026.pdf"));
        assert_eq!(
            fs::read_to_string(vault.asset_path(&added)).unwrap(),
            "offer"
        );
        let again = vault.add_asset(&slug("webshop"), &file).unwrap();
        assert_eq!(again, asset("projects/webshop/assets/Offer 2026 (2).pdf"));
        let third = vault.add_asset(&slug("webshop"), &file).unwrap();
        assert_eq!(third, asset("projects/webshop/assets/Offer 2026 (3).pdf"));
        // No temporary files are left behind.
        assert_eq!(
            fs::read_dir(vault.assets_folder(&slug("webshop")))
                .unwrap()
                .count(),
            3
        );

        assert_eq!(
            edit_error(vault.add_asset(&slug("nothing"), &file).unwrap_err()),
            EditError::UnknownProject(slug("nothing"))
        );
        assert_eq!(
            edit_error(vault.add_asset(&slug("webshop"), &source.0).unwrap_err()),
            EditError::NotAFile(source.0.clone())
        );
        let missing = source.0.join("missing.pdf");
        assert!(matches!(
            vault.add_asset(&slug("webshop"), &missing),
            Err(SaveError::Read(ReadError::Io { .. }))
        ));
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("plan.pdf", 2), "plan (2).pdf");
        assert_eq!(numbered("backup.tar.gz", 3), "backup.tar (3).gz");
        assert_eq!(numbered("Makefile", 2), "Makefile (2)");
    }

    #[test]
    fn rename_assets() {
        let (_dir, vault) = sample_copy();
        let folder = vault.assets_folder(&slug("infra"));
        fs::create_dir_all(folder.join("scans")).unwrap();
        fs::write(folder.join("scans/a.pdf"), "a").unwrap();
        fs::write(folder.join("scans/b.pdf"), "b").unwrap();
        let a = asset("projects/infra/assets/scans/a.pdf");

        let renamed = vault.rename_asset(&a, "c.pdf").unwrap();
        assert_eq!(renamed, asset("projects/infra/assets/scans/c.pdf"));
        assert_eq!(fs::read_to_string(folder.join("scans/c.pdf")).unwrap(), "a");
        assert!(!folder.join("scans/a.pdf").exists());

        assert_eq!(
            edit_error(vault.rename_asset(&a, "d.pdf").unwrap_err()),
            EditError::UnknownAsset(a)
        );
        assert_eq!(
            edit_error(vault.rename_asset(&renamed, "b.pdf").unwrap_err()),
            EditError::AssetExists(asset("projects/infra/assets/scans/b.pdf"))
        );
        assert!(matches!(
            edit_error(vault.rename_asset(&renamed, "../c.pdf").unwrap_err()),
            EditError::InvalidId(_)
        ));
    }
}
