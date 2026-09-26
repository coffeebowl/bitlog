//! Project notes: finding them, reading their links and tags, and creating,
//! renaming and deleting them.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

use chrono::NaiveDate;
use chrono::format::StrftimeItems;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::error::{ReadError, SaveError};
use crate::file::{read_optional, read_text};
use crate::{EditError, NotePath, ProjectSlug, Vault};

/// A wiki link like `[[project-a/deployment]]` in a project note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The target as written, without `|text` or `#heading`, as a byte
    /// range of the note.
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

/// The wiki links in `text`, a note of the project `project`. A target
/// without a project, as in `[[deployment]]`, is a note of `project`.
pub fn wiki_links(text: &str, project: &ProjectSlug) -> Vec<WikiLink> {
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
                    range: start..start + target.len(),
                    note: note_path(target, project),
                })
            }
            _ => None,
        })
        .collect()
}

fn note_path(target: &str, project: &ProjectSlug) -> Option<NotePath> {
    match target.split_once('/') {
        Some((slug, name)) => NotePath::new(slug.parse().ok()?, name).ok(),
        None => NotePath::new(project.clone(), target).ok(),
    }
}

/// The tags in `text`, without `#`, each once, in the order they first
/// appear. A tag is a `#` at the start of a word, followed by letters,
/// digits, `-`, `_` or `/`, not digits only, so that `#123` stays an issue
/// number. Code, front matter and link targets hold no tags.
pub fn tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut code_block = false;
    for event in pulldown_cmark::TextMergeStream::new(parser(text)) {
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::MetadataBlock(_)) => code_block = true,
            Event::End(TagEnd::CodeBlock | TagEnd::MetadataBlock(_)) => code_block = false,
            Event::Text(text) if !code_block => {
                for tag in text_tags(&text) {
                    if !tags.iter().any(|known| known == tag) {
                        tags.push(tag.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    tags
}

fn text_tags(text: &str) -> impl Iterator<Item = &str> {
    let is_tag_char = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/');
    text.split(char::is_whitespace).filter_map(move |word| {
        let rest = word.strip_prefix('#')?;
        let end = rest.find(|c| !is_tag_char(c)).unwrap_or(rest.len());
        let tag = &rest[..end];
        tag.chars().any(|c| !c.is_ascii_digit()).then_some(tag)
    })
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

    pub fn load_note(&self, note: &NotePath) -> Result<String, ReadError> {
        read_text(&self.note_path(note))
    }

    /// The notes of all projects with a wiki link to `target`.
    pub fn notes_linking_to(&self, target: &NotePath) -> Result<Vec<NotePath>, ReadError> {
        let mut linking = Vec::new();
        for project in self.projects() {
            for note in self.notes(&project.slug)? {
                let text = self.load_note(&note)?;
                if wiki_links(&text, &project.slug)
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
        let mut text = self.load_note(note)?;
        let links = wiki_links(&text, note.project());
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
    fn links_and_tags_of_sample_notes() {
        let (_dir, vault) = sample_copy();
        let checkout = note("projects/webshop/notes/checkout-flow.md");
        let text = vault.load_note(&checkout).unwrap();
        let targets: Vec<_> = wiki_links(&text, checkout.project())
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
        assert_eq!(tags(&text), ["checkout"]);

        // The front matter and the code block hold no tags.
        let provider = vault
            .load_note(&note("projects/webshop/notes/payment-provider.md"))
            .unwrap();
        assert_eq!(tags(&provider), ["payments"]);
    }

    #[test]
    fn link_forms() {
        let project = slug("webshop");
        let text = "[[infra/deployment#Steps|how]] [[Auth Middleware]] [[a/b/c]] \
                    `[[infra/in-code]]` [[]]";
        let links = wiki_links(text, &project);
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
    }

    #[test]
    fn tag_rules() {
        let text = "#start, see #123 and #v2 or #a/b-c_d.\n\
                    word#no https://x.example/#frag *#bold*\n\n\
                    # Heading #in-heading\n\n    #indented-code\n";
        assert_eq!(tags(text), ["start", "v2", "a/b-c_d", "bold", "in-heading"]);
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
            vault.load_note(&created).unwrap(),
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
        assert_eq!(vault.load_note(&created).unwrap(), "");
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
        let provider_text = vault.load_note(&provider).unwrap()
            + "[[checkout-flow#Payment|pay]] `[[webshop/checkout-flow]]`\n";
        fs::write(vault.note_path(&provider), &provider_text).unwrap();

        let renamed = vault.rename_note(&checkout, "Checkout", true).unwrap();
        assert_eq!(renamed, note("projects/webshop/notes/Checkout.md"));
        assert!(!vault.note_path(&checkout).exists());
        assert!(
            vault
                .load_note(&renamed)
                .unwrap()
                .starts_with("# Checkout flow\n")
        );
        assert!(
            vault
                .load_note(&deployment)
                .unwrap()
                .contains("see [[webshop/Checkout]].")
        );
        assert_eq!(
            vault.load_note(&provider).unwrap(),
            provider_text
                .replace(
                    "see [[webshop/checkout-flow]] for",
                    "see [[webshop/Checkout]] for"
                )
                .replace("[[checkout-flow#Payment|pay]]", "[[Checkout#Payment|pay]]")
        );

        // Without updating, links stay as they are.
        let before = vault.load_note(&deployment).unwrap();
        vault.rename_note(&renamed, "checkout-flow", false).unwrap();
        assert_eq!(vault.load_note(&deployment).unwrap(), before);

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
