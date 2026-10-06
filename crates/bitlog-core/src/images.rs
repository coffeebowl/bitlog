//! Images that texts show, linked by their path relative to the file of the
//! text, see "Images" in the format spec.

use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use chrono::NaiveDateTime;

use crate::assets::numbered;
use crate::error::{EditError, ReadError, SaveError};
use crate::file::{read_folder, write_atomic};
use crate::markdown::{MarkdownMode, has_scheme, markdown_formatting};

/// The folder that images are added to, in the vault at `root`.
pub(crate) fn images_folder(root: &Path) -> PathBuf {
    root.join("images")
}

/// What is wrong with an image link, as `bitlog doctor` finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProblem {
    /// It leads to a file of the vault that is missing.
    Missing,
    /// It is an absolute path or leads out of the vault, so it shows no
    /// image, and on other devices it would lead elsewhere.
    OutsideVault,
}

/// Copies the image file `source` into the images folder of the vault at
/// `root`, unless an image there has the same content. It keeps its name,
/// with a number added if the name is taken, as in `login (2).png`. A file
/// of the vault is not copied: it is the image. Returns where the image is.
pub fn add_image(root: &Path, source: &Path) -> Result<PathBuf, SaveError> {
    if !source.is_file() {
        return Err(EditError::NotAFile(source.to_owned()).into());
    }
    if let Some(inside) = in_vault(root, source) {
        return Ok(inside);
    }
    let content = fs::read(source).map_err(|source_error| ReadError::Io {
        path: source.to_owned(),
        source: source_error,
    })?;
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    put_image(root, &name, &content)
}

/// Saves an image pasted at `now`, with its content in `png`, in the
/// images folder of the vault at `root`, like [`add_image`]. Named after
/// the time, as in `2026-10-06 12-34-56.png`, as it has no name; without
/// `:`, which some systems do not take in file names.
pub fn add_pasted_image(root: &Path, png: &[u8], now: NaiveDateTime) -> Result<PathBuf, SaveError> {
    put_image(root, &now.format("%Y-%m-%d %H-%M-%S.png").to_string(), png)
}

/// Where `path` is in the vault at `root`, as a path below `root`, if it
/// lies there, links followed.
fn in_vault(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path
        .canonicalize()
        .ok()?
        .strip_prefix(root.canonicalize().ok()?)
        .ok()?
        .to_owned();
    Some(root.join(relative))
}

/// Puts `content` into the images folder of the vault at `root` as `name`,
/// see [`add_image`].
fn put_image(root: &Path, name: &str, content: &[u8]) -> Result<PathBuf, SaveError> {
    // The first by path, if several have the content.
    let same = image_files(root)?.into_iter().find(|file| {
        fs::metadata(file).is_ok_and(|metadata| metadata.len() == content.len() as u64)
            && fs::read(file).is_ok_and(|other| other == content)
    });
    if let Some(same) = same {
        return Ok(same);
    }
    let folder = images_folder(root);
    let mut path = folder.join(name);
    let mut number = 2;
    while path.exists() {
        path = folder.join(numbered(name, number));
        number += 1;
    }
    write_atomic(&path, content)?;
    Ok(path)
}

/// The link to the `image` that a text in `file` shows it with, as BitLog
/// writes it: `![](../../../images/login.png)`, the target in angle brackets
/// if it has spaces or parentheses.
pub fn image_link(file: &Path, image: &Path) -> String {
    let from: Vec<_> = file
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .collect();
    let to: Vec<_> = image.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let parts: Vec<String> = std::iter::repeat_n("..".to_owned(), from.len() - common)
        .chain(
            to[common..]
                .iter()
                .map(|part| part.as_os_str().to_string_lossy().into_owned()),
        )
        .collect();
    let target = parts.join("/");
    if target.contains([' ', '(', ')']) {
        format!("![](<{target}>)")
    } else {
        format!("![]({target})")
    }
}

/// The image links of `text`, in `mode`, as the text is saved in `file` of
/// the vault at `root`: those with a problem, and the images the others
/// show.
pub(crate) fn check_image_links(
    root: &Path,
    file: &Path,
    text: &str,
    mode: MarkdownMode,
) -> (Vec<(Range<usize>, ImageProblem)>, Vec<PathBuf>) {
    let mut problems = Vec::new();
    let mut shown = Vec::new();
    for image in markdown_formatting(text, mode).images {
        match image_path(root, file, &image.destination) {
            Some(path) if path.is_file() => shown.push(path),
            Some(_) => problems.push((image.range, ImageProblem::Missing)),
            None if !has_scheme(&image.destination) => {
                problems.push((image.range, ImageProblem::OutsideVault));
            }
            // Web addresses show no image, as meant.
            None => {}
        }
    }
    (problems, shown)
}

/// The files in the images folder of the vault at `root` and its
/// subfolders, sorted, but for hidden ones.
pub(crate) fn image_files(root: &Path) -> Result<Vec<PathBuf>, ReadError> {
    let mut files = Vec::new();
    let mut folders = vec![images_folder(root)];
    while let Some(folder) = folders.pop() {
        for entry in read_folder(&folder)? {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            // Links to folders are not followed, as they may lead in circles.
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(entry.path()),
                Ok(_) => files.push(entry.path()),
                Err(_) => {}
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The groups of `files` with the same content, each sorted, of at least
/// two. Only files of the same size are read.
pub(crate) fn duplicates(files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    let mut by_size: HashMap<u64, Vec<&PathBuf>> = HashMap::new();
    for file in files {
        if let Ok(metadata) = fs::metadata(file) {
            by_size.entry(metadata.len()).or_default().push(file);
        }
    }
    let mut groups: Vec<Vec<PathBuf>> = Vec::new();
    for same_size in by_size.into_values().filter(|files| files.len() > 1) {
        // Few files have the same size, so comparing them is quick.
        let mut by_content: Vec<(Vec<u8>, Vec<PathBuf>)> = Vec::new();
        for file in same_size {
            let Ok(content) = fs::read(file) else {
                continue;
            };
            match by_content.iter_mut().find(|(other, _)| *other == content) {
                Some((_, group)) => group.push(file.clone()),
                None => by_content.push((content, vec![file.clone()])),
            }
        }
        groups.extend(
            by_content
                .into_iter()
                .map(|(_, group)| group)
                .filter(|group| group.len() > 1),
        );
    }
    for group in &mut groups {
        group.sort();
    }
    groups.sort();
    groups
}

/// Where the image is that a text in the file `file` of the vault at `root`
/// links to as `destination`, like `../../../images/login.png`. Only
/// relative paths that stay in the vault lead to an image: web addresses,
/// absolute paths and paths out of the vault lead nowhere.
pub fn image_path(root: &Path, file: &Path, destination: &str) -> Option<PathBuf> {
    if destination.starts_with(['/', '\\']) || has_scheme(destination) {
        return None;
    }
    let folder = file.parent()?.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in folder.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_str()?.to_owned());
    }
    for part in percent_decoded(destination)?.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part.to_owned()),
        }
    }
    Some(
        parts
            .iter()
            .fold(root.to_owned(), |path, part| path.join(part)),
    )
}

/// `text` with `%20` and the like decoded, as Markdown links may write
/// spaces. A `%` without two hex digits after it stays. `None` if the
/// decoded bytes are no UTF-8.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .filter(|digits| digits.chars().all(|c| c.is_ascii_hexdigit()))
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                i += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::TempDir;

    fn resolve(file: &str, destination: &str) -> Option<PathBuf> {
        image_path(
            Path::new("/vault"),
            &Path::new("/vault").join(file),
            destination,
        )
    }

    #[test]
    fn days_and_notes_reach_the_images_alike() {
        for file in [
            "daily/2026/10/2026-10-06.md",
            "projects/infra/notes/deployment.md",
        ] {
            assert_eq!(
                resolve(file, "../../../images/login.png"),
                Some(PathBuf::from("/vault/images/login.png"))
            );
        }
        assert_eq!(
            resolve(
                "projects/infra/notes/deployment.md",
                "../assets/scans/./rack.jpg"
            ),
            Some(PathBuf::from("/vault/projects/infra/assets/scans/rack.jpg"))
        );
    }

    #[test]
    fn spaces_may_be_encoded() {
        let image = Some(PathBuf::from("/vault/images/2026-10-06 12-34-56.png"));
        let file = "daily/2026/10/2026-10-06.md";
        assert_eq!(
            resolve(file, "../../../images/2026-10-06%2012-34-56.png"),
            image
        );
        assert_eq!(
            resolve(file, "../../../images/2026-10-06 12-34-56.png"),
            image
        );
        assert_eq!(
            resolve(file, "100%.png"),
            Some(PathBuf::from("/vault/daily/2026/10/100%.png"))
        );
        assert_eq!(resolve(file, "%FF.png"), None);
    }

    #[test]
    fn only_paths_within_the_vault_lead_to_images() {
        let file = "daily/2026/10/2026-10-06.md";
        for destination in [
            "../../../../secret.png",
            "/home/anna/login.png",
            "https://example.com/login.png",
            "file:///vault/images/login.png",
        ] {
            assert_eq!(resolve(file, destination), None, "{destination}");
        }
        assert_eq!(
            image_path(Path::new("/vault"), Path::new("/elsewhere/a.md"), "a.png"),
            None
        );
    }

    #[test]
    fn links_lead_to_their_images() {
        let day = Path::new("/vault/daily/2026/10/2026-10-06.md");
        let image = Path::new("/vault/images/login.png");
        assert_eq!(image_link(day, image), "![](../../../images/login.png)");
        let note = Path::new("/vault/projects/infra/notes/deployment.md");
        let spaced = Path::new("/vault/images/2026-10-06 12-34-56.png");
        let link = image_link(note, spaced);
        assert_eq!(link, "![](<../../../images/2026-10-06 12-34-56.png>)");
        let formatting = markdown_formatting(&link, MarkdownMode::Full);
        assert_eq!(
            image_path(Path::new("/vault"), note, &formatting.images[0].destination),
            Some(spaced.to_owned())
        );
        assert_eq!(
            image_link(note, Path::new("/vault/projects/infra/assets/rack.jpg")),
            "![](../assets/rack.jpg)"
        );
    }

    #[test]
    fn images_keep_their_names_and_are_not_added_twice() {
        let root = TempDir::new();
        let sources = TempDir::new();
        fs::create_dir_all(&sources.0).unwrap();
        let login = sources.0.join("2026-10-06 login.png");
        fs::write(&login, "login").unwrap();
        let other = sources.0.join("other").join("2026-10-06 login.png");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, "other").unwrap();
        let folder = images_folder(&root.0);

        let added = add_image(&root.0, &login).unwrap();
        assert_eq!(added, folder.join("2026-10-06 login.png"));
        assert_eq!(fs::read_to_string(&added).unwrap(), "login");
        // The same content again is the same image.
        assert_eq!(add_image(&root.0, &login).unwrap(), added);
        assert_eq!(
            add_image(&root.0, &other).unwrap(),
            folder.join("2026-10-06 login (2).png")
        );

        let now =
            NaiveDateTime::parse_from_str("2026-10-06 12:34:56", "%Y-%m-%d %H:%M:%S").unwrap();
        let pasted = add_pasted_image(&root.0, b"pixels", now).unwrap();
        assert_eq!(pasted, folder.join("2026-10-06 12-34-56.png"));
        assert_eq!(add_pasted_image(&root.0, b"login", now).unwrap(), added);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 3);

        assert!(matches!(
            add_image(&root.0, &sources.0),
            Err(SaveError::Edit(EditError::NotAFile(_)))
        ));

        // A copy in the vault is the image, not the first of its content.
        let copy = folder.join("copy of login.png");
        fs::copy(&added, &copy).unwrap();
        assert_eq!(add_image(&root.0, &copy).unwrap(), copy);
        let asset = root.0.join("projects/infra/assets/rack.png");
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(&asset, "rack").unwrap();
        assert_eq!(add_image(&root.0, &asset).unwrap(), asset);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 4);
    }

    #[test]
    fn duplicates_have_the_same_content() {
        let root = TempDir::new();
        let folder = images_folder(&root.0);
        fs::create_dir_all(folder.join("old")).unwrap();
        for (name, content) in [
            ("a.png", "same"),
            ("b.png", "diff"),
            ("old/c.png", "same"),
            (".hidden.png", "same"),
            ("d.png", "other content"),
        ] {
            fs::write(folder.join(name), content).unwrap();
        }
        let files = image_files(&root.0).unwrap();
        assert_eq!(
            files,
            [
                folder.join("a.png"),
                folder.join("b.png"),
                folder.join("d.png"),
                folder.join("old/c.png")
            ]
        );
        assert_eq!(
            duplicates(&files),
            [vec![folder.join("a.png"), folder.join("old/c.png")]]
        );
    }
}
