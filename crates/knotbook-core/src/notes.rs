//! Project notes: finding them, reading their links, and creating,
//! renaming and deleting them.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

use chrono::NaiveDate;
use chrono::format::StrftimeItems;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};

use crate::error::{ReadError, SaveError};
use crate::file::{content_hash, read_optional, read_text};
use crate::{EditError, NotePath, ProjectSlug, Vault};

/// A project note as read, remembering the file's content to notice
/// changes made elsewhere before saving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFile {
    pub path: NotePath,
    pub text: String,
    hash: u64,
}

impl NoteFile {
    fn new(path: NotePath, text: String) -> Self {
        let hash = content_hash(&text);
        Self { path, text, hash }
    }
}

/// What saving a note came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedNote {
    Saved(NoteFile),
    /// The note was changed elsewhere since it was read, so nothing was
    /// written. Holds the note as it is now.
    Conflict(NoteFile),
}

/// A wiki link like `[[project-a/deployment]]` in a note or day file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The whole link, brackets included, as a byte range of the text.
    pub span: Range<usize>,
    /// The target as written, without `|text` or `#heading`, as a byte
    /// range of the text.
    pub range: Range<usize>,
    /// The note the link points to, whether it exists or not. `None` if
    /// the target is no valid note path.
    pub note: Option<NotePath>,
}

impl WikiLink {
    /// Whether the target leaves out the project, as in `[[deployment]]`.
    fn is_short(&self, text: &str) -> bool {
        !text[self.range.clone()].contains('/')
    }
}

/// The wiki links in `text`, a note or block text of the project `project`.
/// A target without a project, as in `[[deployment]]`, is a note of
/// `project`, and points nowhere in text of no project, such as a day note.
pub fn wiki_links(text: &str, project: Option<&ProjectSlug>) -> Vec<WikiLink> {
    parser(text)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::Link {
                link_type: LinkType::WikiLink { .. },
                dest_url,
                ..
            }) => {
                let target = dest_url.split('#').next().unwrap_or_default();
                let start = range.start + "[[".len();
                // pulldown-cmark takes the target straight from the text.
                text[start..].starts_with(target).then(|| WikiLink {
                    span: range,
                    range: start..start + target.len(),
                    note: note_path(target, project),
                })
            }
            _ => None,
        })
        .collect()
}

fn note_path(target: &str, project: Option<&ProjectSlug>) -> Option<NotePath> {
    match target.split_once('/') {
        Some((slug, name)) => NotePath::new(slug.parse().ok()?, name).ok(),
        None => NotePath::new(project?.clone(), target).ok(),
    }
}

fn parser(text: &str) -> Parser<'_> {
    Parser::new_ext(
        text,
        Options::ENABLE_WIKILINKS
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS,
    )
}

/// Fills in the placeholders of a note template: `{{title}}`, `{{project}}`
/// and `{{date:FORMAT}}` with a strftime format, or `{{date}}` for
/// `YYYY-MM-DD`. Unknown placeholders and invalid formats stay as they are.
fn instantiate(template: &str, title: &str, project: &str, today: NaiveDate) -> String {
    let mut note = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}").map(|end| start + end) else {
            break;
        };
        note.push_str(&rest[..start]);
        let placeholder = &rest[start + 2..end];
        let format = match placeholder {
            "date" => Some("%Y-%m-%d"),
            _ => placeholder.strip_prefix("date:"),
        };
        let value = match (placeholder, format) {
            ("title", _) => Some(title.to_owned()),
            ("project", _) => Some(project.to_owned()),
            (_, Some(format)) => StrftimeItems::new(format)
                .parse()
                .ok()
                .map(|items| today.format_with_items(items.iter()).to_string()),
            _ => None,
        };
        note.push_str(value.as_deref().unwrap_or(&rest[start..end + 2]));
        rest = &rest[end + 2..];
    }
    note.push_str(rest);
    note
}

impl Vault {
    /// Where the note `note` lives.
    pub fn note_path(&self, note: &NotePath) -> PathBuf {
        self.root()
            .join("projects")
            .join(note.project().as_str())
            .join("notes")
            .join(format!("{}.md", note.name()))
    }

    /// The notes of the project `project`, sorted by name.
    pub fn notes(&self, project: &ProjectSlug) -> Result<Vec<NotePath>, ReadError> {
        let folder = self
            .root()
            .join("projects")
            .join(project.as_str())
            .join("notes");
        let io_error = |source| ReadError::Io {
            path: folder.clone(),
            source,
        };
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error(err)),
        };
        let mut notes = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let note = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".md"))
                .and_then(|name| NotePath::new(project.clone(), name).ok());
            if let Some(note) = note
                && entry.path().is_file()
            {
                notes.push(note);
            }
        }
        notes.sort();
        Ok(notes)
    }

    pub fn load_note(&self, note: &NotePath) -> Result<NoteFile, ReadError> {
        let text = read_text(&self.note_path(note))?;
        Ok(NoteFile::new(note.clone(), text))
    }

    /// Saves `text` as the note of `file`. If the note was changed elsewhere
    /// since `file` was read, nothing is written and the note as it is now
    /// comes back as a conflict, unless it holds `text` already. A note
    /// removed elsewhere is created again, so that nothing typed gets lost.
    pub fn save_note(&self, file: &NoteFile, text: &str) -> Result<SavedNote, SaveError> {
        let path = self.note_path(&file.path);
        if let Some(current) = read_optional(&path)? {
            if current == text {
                return Ok(SavedNote::Saved(NoteFile::new(file.path.clone(), current)));
            }
            if content_hash(&current) != file.hash {
                return Ok(SavedNote::Conflict(NoteFile::new(
                    file.path.clone(),
                    current,
                )));
            }
        }
        self.write(&path, text)?;
        Ok(SavedNote::Saved(NoteFile::new(
            file.path.clone(),
            text.to_owned(),
        )))
    }

    /// The notes of all projects with a wiki link to `target`.
    pub fn notes_linking_to(&self, target: &NotePath) -> Result<Vec<NotePath>, ReadError> {
        let mut linking = Vec::new();
        for project in self.projects() {
            for note in self.notes(&project.slug)? {
                let file = self.load_note(&note)?;
                if wiki_links(&file.text, Some(&project.slug))
                    .iter()
                    .any(|link| link.note.as_ref() == Some(target))
                {
                    linking.push(note);
                }
            }
        }
        Ok(linking)
    }

    /// Creates the note `name` in the project `project` from the vault's
    /// note template, or empty without one.
    pub fn create_note(
        &self,
        project: &ProjectSlug,
        name: &str,
        today: NaiveDate,
    ) -> Result<NotePath, SaveError> {
        let project_name = &self
            .project(project)
            .ok_or_else(|| EditError::UnknownProject(project.clone()))?
            .name;
        let note = NotePath::new(project.clone(), name).map_err(EditError::from)?;
        let path = self.note_path(&note);
        if path.exists() {
            return Err(EditError::NoteExists(note).into());
        }
        let template = match &self.config().defaults.note_template {
            Some(template) => read_optional(&self.root().join(template))?.unwrap_or_default(),
            None => String::new(),
        };
        self.write(&path, &instantiate(&template, name, project_name, today))?;
        Ok(note)
    }

    /// Renames the note `note` to `name`, within its project. With
    /// `update_links`, wiki links to it in all notes are changed to the new
    /// name, keeping their `|text` and `#heading`.
    pub fn rename_note(
        &self,
        note: &NotePath,
        name: &str,
        update_links: bool,
    ) -> Result<NotePath, SaveError> {
        let renamed = NotePath::new(note.project().clone(), name).map_err(EditError::from)?;
        let from = self.note_path(note);
        let to = self.note_path(&renamed);
        if !from.is_file() {
            return Err(EditError::UnknownNote(note.clone()).into());
        }
        if to.exists() {
            return Err(EditError::NoteExists(renamed).into());
        }
        let text = read_text(&from)?;
        self.record_write(&to, Some(&text));
        self.record_write(&from, None);
        fs::rename(&from, &to).map_err(|source| SaveError::Write { path: to, source })?;
        if update_links {
            // The renamed note itself included, for links to itself.
            for linking in self.notes_linking_to(note)? {
                self.relink(&linking, note, &renamed)?;
            }
        }
        Ok(renamed)
    }

    /// Changes the wiki links to `from` in the note `note` to point to `to`.
    fn relink(&self, note: &NotePath, from: &NotePath, to: &NotePath) -> Result<(), SaveError> {
        let mut text = self.load_note(note)?.text;
        let links = wiki_links(&text, Some(note.project()));
        // From the end, so that the ranges before stay valid.
        for link in links.iter().rev() {
            if link.note.as_ref() != Some(from) {
                continue;
            }
            let target = if link.is_short(&text) {
                to.name().to_owned()
            } else {
                format!("{}/{}", to.project(), to.name())
            };
            text.replace_range(link.range.clone(), &target);
        }
        self.write(&self.note_path(note), &text)
    }

    /// Deletes the note `note` for good.
    pub fn delete_note(&self, note: &NotePath) -> Result<(), SaveError> {
        let path = self.note_path(note);
        self.record_write(&path, None);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                Err(EditError::UnknownNote(note.clone()).into())
            }
            Err(source) => Err(SaveError::Write { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::sample_copy;

    fn slug(slug: &str) -> ProjectSlug {
        slug.parse().unwrap()
    }

    fn note(path: &str) -> NotePath {
        path.parse().unwrap()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 26).unwrap()
    }

    #[test]
    fn list_notes() {
        let (_dir, vault) = sample_copy();
        assert_eq!(
            vault.notes(&slug("webshop")).unwrap(),
            [
                note("projects/webshop/notes/checkout-flow.md"),
                note("projects/webshop/notes/payment-provider.md")
            ]
        );
        assert!(vault.notes(&slug("pause")).unwrap().is_empty());
    }

    #[test]
    fn links_of_sample_notes() {
        let (_dir, vault) = sample_copy();
        let checkout = note("projects/webshop/notes/checkout-flow.md");
        let text = vault.load_note(&checkout).unwrap().text;
        let targets: Vec<_> = wiki_links(&text, Some(checkout.project()))
            .into_iter()
            .map(|link| link.note.unwrap().to_string())
            .collect();
        assert_eq!(
            targets,
            [
                "projects/webshop/notes/payment-provider.md",
                "projects/infra/notes/deployment.md"
            ]
        );
    }

    #[test]
    fn link_forms() {
        let project = slug("webshop");
        let text = "[[infra/deployment#Steps|how]] [[Auth Middleware]] [[a/b/c]] \
                    `[[infra/in-code]]` [[]]";
        let links = wiki_links(text, Some(&project));
        let found: Vec<_> = links
            .iter()
            .map(|link| (&text[link.range.clone()], link.note.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "infra/deployment",
                    Some(note("projects/infra/notes/deployment.md"))
                ),
                (
                    "Auth Middleware",
                    Some(note("projects/webshop/notes/Auth Middleware.md"))
                ),
                ("a/b/c", None),
            ]
        );
        let spans: Vec<_> = links.iter().map(|link| &text[link.span.clone()]).collect();
        assert_eq!(
            spans,
            [
                "[[infra/deployment#Steps|how]]",
                "[[Auth Middleware]]",
                "[[a/b/c]]"
            ]
        );
    }

    #[test]
    fn short_links_need_a_project() {
        let links = wiki_links("[[infra/deployment]] [[deployment]]", None);
        let notes: Vec<_> = links.into_iter().map(|link| link.note).collect();
        assert_eq!(
            notes,
            [Some(note("projects/infra/notes/deployment.md")), None]
        );
    }

    #[test]
    fn instantiate_template() {
        let template = "# {{title}}\n\n{{date}} {{date:%d.%m.%Y}} in {{project}} \
                        {{unknown}} {{date:%Q}} {{title";
        assert_eq!(
            instantiate(template, "Auth", "Project A", today()),
            "# Auth\n\n2026-09-26 26.09.2026 in Project A {{unknown}} {{date:%Q}} {{title"
        );
    }

    #[test]
    fn create_note_from_template() {
        let (_dir, vault) = sample_copy();
        let created = vault
            .create_note(&slug("webshop"), "Auth middleware", today())
            .unwrap();
        assert_eq!(created, note("projects/webshop/notes/Auth middleware.md"));
        assert_eq!(
            vault.load_note(&created).unwrap().text,
            "# Auth middleware\n\nCreated 2026-09-26 in Webshop.\n"
        );

        let err = |result: Result<NotePath, SaveError>| match result.unwrap_err() {
            SaveError::Edit(err) => err,
            err => panic!("{err}"),
        };
        assert_eq!(
            err(vault.create_note(&slug("webshop"), "Auth middleware", today())),
            EditError::NoteExists(created)
        );
        assert!(matches!(
            err(vault.create_note(&slug("webshop"), ".hidden", today())),
            EditError::InvalidId(_)
        ));
        assert_eq!(
            err(vault.create_note(&slug("nothing"), "Note", today())),
            EditError::UnknownProject(slug("nothing"))
        );
    }

    #[test]
    fn create_note_without_template() {
        let (dir, vault) = sample_copy();
        fs::remove_file(dir.0.join("templates/note.md")).unwrap();
        let created = vault.create_note(&slug("infra"), "Empty", today()).unwrap();
        assert_eq!(vault.load_note(&created).unwrap().text, "");
    }

    #[test]
    fn rename_note_and_links() {
        let (_dir, vault) = sample_copy();
        let checkout = note("projects/webshop/notes/checkout-flow.md");
        let deployment = note("projects/infra/notes/deployment.md");
        let provider = note("projects/webshop/notes/payment-provider.md");
        assert_eq!(
            vault.notes_linking_to(&checkout).unwrap(),
            [deployment.clone(), provider.clone()]
        );

        // Short links, links to headings and links in code.
        let provider_text = vault.load_note(&provider).unwrap().text
            + "[[checkout-flow#Payment|pay]] `[[webshop/checkout-flow]]`\n";
        fs::write(vault.note_path(&provider), &provider_text).unwrap();

        let renamed = vault.rename_note(&checkout, "Checkout", true).unwrap();
        assert_eq!(renamed, note("projects/webshop/notes/Checkout.md"));
        assert!(!vault.note_path(&checkout).exists());
        assert!(
            vault
                .load_note(&renamed)
                .unwrap()
                .text
                .starts_with("# Checkout flow\n")
        );
        assert!(
            vault
                .load_note(&deployment)
                .unwrap()
                .text
                .contains("see [[webshop/Checkout]].")
        );
        assert_eq!(
            vault.load_note(&provider).unwrap().text,
            provider_text
                .replace(
                    "see [[webshop/checkout-flow]] for",
                    "see [[webshop/Checkout]] for"
                )
                .replace("[[checkout-flow#Payment|pay]]", "[[Checkout#Payment|pay]]")
        );

        // Without updating, links stay as they are.
        let before = vault.load_note(&deployment).unwrap().text;
        vault.rename_note(&renamed, "checkout-flow", false).unwrap();
        assert_eq!(vault.load_note(&deployment).unwrap().text, before);

        // Names only clash within a project.
        vault.rename_note(&provider, "deployment", false).unwrap();
        let err = vault
            .rename_note(&checkout, "deployment", false)
            .unwrap_err();
        assert!(
            matches!(err, SaveError::Edit(EditError::NoteExists(_))),
            "{err}"
        );
    }

    #[test]
    fn save_note() {
        let (_dir, vault) = sample_copy();
        let file = vault
            .load_note(&note("projects/infra/notes/deployment.md"))
            .unwrap();
        let path = vault.note_path(&file.path);
        let SavedNote::Saved(saved) = vault.save_note(&file, "Mine.\n").unwrap() else {
            panic!("the note was not changed elsewhere");
        };
        assert_eq!(fs::read_to_string(&path).unwrap(), "Mine.\n");

        // Changed elsewhere: nothing is written.
        fs::write(&path, "Theirs.\n").unwrap();
        let SavedNote::Conflict(theirs) = vault.save_note(&saved, "Mine again.\n").unwrap() else {
            panic!("the note was changed elsewhere");
        };
        assert_eq!(theirs.text, "Theirs.\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "Theirs.\n");
        // Unless both sides hold the same.
        assert!(matches!(
            vault.save_note(&saved, "Theirs.\n").unwrap(),
            SavedNote::Saved(_)
        ));
        // Keeping one's own version saves over the other one.
        assert!(matches!(
            vault.save_note(&theirs, "Mine again.\n").unwrap(),
            SavedNote::Saved(_)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "Mine again.\n");

        // Removed elsewhere: created again.
        fs::remove_file(&path).unwrap();
        vault.save_note(&theirs, "Back.\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "Back.\n");
    }

    #[test]
    fn delete_note() {
        let (_dir, vault) = sample_copy();
        let deployment = note("projects/infra/notes/deployment.md");
        vault.delete_note(&deployment).unwrap();
        assert!(vault.notes(&slug("infra")).unwrap().is_empty());
        let err = vault.delete_note(&deployment).unwrap_err();
        assert!(
            matches!(err, SaveError::Edit(EditError::UnknownNote(_))),
            "{err}"
        );
    }
}
