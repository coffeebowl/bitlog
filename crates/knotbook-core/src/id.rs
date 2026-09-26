//! Validated identifiers used throughout the vault.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// A string that does not match the rules of an identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} {value:?}, expected {expected}")]
pub struct InvalidId {
    kind: &'static str,
    value: String,
    expected: &'static str,
}

impl InvalidId {
    fn new(kind: &'static str, value: &str, expected: &'static str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
            expected,
        }
    }
}

const SLUG_RULE: &str = "lowercase letters, digits and single hyphens";
const SHORT_ID_RULE: &str = "4 characters from a-z and 0-9";
const SHORT_ID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const SHORT_ID_LEN: usize = 4;
const NOTE_NAME_RULE: &str = "a file name without \"/\" or \"\\\" that does not start with \".\"";
const NOTE_PATH_RULE: &str = "projects/<slug>/notes/<name>.md";

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn is_short_id(value: &str) -> bool {
    value.len() == SHORT_ID_LEN && value.bytes().all(|b| SHORT_ID_ALPHABET.contains(&b))
}

/// Deserializes a string and validates it through `FromStr`.
fn deserialize_parsed<'de, D: Deserializer<'de>, T: FromStr<Err = InvalidId>>(
    deserializer: D,
) -> Result<T, D::Error> {
    String::deserialize(deserializer)?
        .parse()
        .map_err(serde::de::Error::custom)
}

fn random_short_id() -> String {
    (0..SHORT_ID_LEN)
        .map(|_| char::from(SHORT_ID_ALPHABET[fastrand::usize(..SHORT_ID_ALPHABET.len())]))
        .collect()
}

/// Identifies a project, and names its folder below `projects/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectSlug(String);

impl ProjectSlug {
    /// A slug made from the project name `name`, as in "Web Shop" →
    /// `web-shop`, or `None` if nothing of it fits.
    pub fn from_name(name: &str) -> Option<Self> {
        let mut slug = String::new();
        let mut hyphen = false;
        for c in name.chars().flat_map(char::to_lowercase) {
            let mut buffer = [0; 4];
            let part: &str = match c {
                'a'..='z' | '0'..='9' => c.encode_utf8(&mut buffer),
                'ä' => "ae",
                'ö' => "oe",
                'ü' => "ue",
                'ß' => "ss",
                _ => {
                    hyphen = !slug.is_empty();
                    continue;
                }
            };
            if hyphen {
                slug.push('-');
                hyphen = false;
            }
            slug.push_str(part);
        }
        slug.parse().ok()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProjectSlug {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        if is_slug(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidId::new("project slug", value, SLUG_RULE))
        }
    }
}

impl fmt::Display for ProjectSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProjectSlug {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_parsed(deserializer)
    }
}

/// A key of the `[locations]` table in `knotbook.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocationKey(String);

impl LocationKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for LocationKey {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        if is_slug(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidId::new("location key", value, SLUG_RULE))
        }
    }
}

impl fmt::Display for LocationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for LocationKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_parsed(deserializer)
    }
}

/// Identifies a block within its day.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(String);

impl BlockId {
    /// Creates a random id for which `is_taken` returns false.
    pub fn generate(is_taken: impl Fn(&Self) -> bool) -> Self {
        loop {
            let id = Self(random_short_id());
            if !is_taken(&id) {
                return id;
            }
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BlockId {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        if is_short_id(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidId::new("block id", value, SHORT_ID_RULE))
        }
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BlockId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_parsed(deserializer)
    }
}

/// Identifies a task within `tasks.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(String);

impl TaskId {
    /// Creates a random id for which `is_taken` returns false.
    pub fn generate(is_taken: impl Fn(&Self) -> bool) -> Self {
        loop {
            let id = Self(random_short_id());
            if !is_taken(&id) {
                return id;
            }
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TaskId {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        if is_short_id(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidId::new("task id", value, SHORT_ID_RULE))
        }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TaskId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_parsed(deserializer)
    }
}

/// Identifies a project note by its path relative to the vault,
/// `projects/<slug>/notes/<name>.md`, always with `/` as separator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NotePath {
    project: ProjectSlug,
    name: String,
}

impl NotePath {
    /// Creates the path of the note `name` (without `.md`) in `project`.
    pub fn new(project: ProjectSlug, name: &str) -> Result<Self, InvalidId> {
        if is_note_name(name) {
            Ok(Self {
                project,
                name: name.to_owned(),
            })
        } else {
            Err(InvalidId::new("note name", name, NOTE_NAME_RULE))
        }
    }

    pub fn project(&self) -> &ProjectSlug {
        &self.project
    }

    /// The file name without `.md`.
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn is_note_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
}

impl FromStr for NotePath {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        let invalid = || InvalidId::new("note path", value, NOTE_PATH_RULE);
        let rest = value.strip_prefix("projects/").ok_or_else(invalid)?;
        let (project, rest) = rest.split_once("/notes/").ok_or_else(invalid)?;
        let name = rest.strip_suffix(".md").ok_or_else(invalid)?;
        let project = project.parse().map_err(|_| invalid())?;
        Self::new(project, name).map_err(|_| invalid())
    }
}

impl fmt::Display for NotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "projects/{}/notes/{}.md", self.project, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        for valid in ["webshop", "project-a", "a", "2026-q3", "a-b-c"] {
            assert_eq!(valid.parse::<ProjectSlug>().unwrap().as_str(), valid);
            assert_eq!(valid.parse::<LocationKey>().unwrap().as_str(), valid);
        }
        for invalid in [
            "", "-a", "a-", "a--b", "Webshop", "web shop", "web_shop", "ü", "a/b",
        ] {
            assert!(invalid.parse::<ProjectSlug>().is_err(), "{invalid:?}");
            assert!(invalid.parse::<LocationKey>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn slugs_from_names() {
        let slug = |name| ProjectSlug::from_name(name).map(|slug| slug.0);
        assert_eq!(slug("Web Shop").as_deref(), Some("web-shop"));
        assert_eq!(
            slug("  Größe -- 2026 Q3! ").as_deref(),
            Some("groesse-2026-q3")
        );
        assert_eq!(slug("infra").as_deref(), Some("infra"));
        assert_eq!(slug(" – ?"), None);
    }

    #[test]
    fn short_ids() {
        for valid in ["k7f3", "aaaa", "0000"] {
            assert_eq!(valid.parse::<BlockId>().unwrap().as_str(), valid);
            assert_eq!(valid.parse::<TaskId>().unwrap().as_str(), valid);
        }
        for invalid in ["", "k7f", "k7f3a", "K7F3", "k7-3", "k7 3", "ä7f3"] {
            assert!(invalid.parse::<BlockId>().is_err(), "{invalid:?}");
            assert!(invalid.parse::<TaskId>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn generated_ids_are_valid_and_free() {
        let taken: BlockId = "k7f3".parse().unwrap();
        for _ in 0..1000 {
            let id = BlockId::generate(|id| *id == taken);
            assert_ne!(id, taken);
            assert!(id.as_str().parse::<BlockId>().is_ok());
            assert!(
                TaskId::generate(|_| false)
                    .as_str()
                    .parse::<TaskId>()
                    .is_ok()
            );
        }
    }

    #[test]
    fn note_paths() {
        let path: NotePath = "projects/webshop/notes/checkout-flow.md".parse().unwrap();
        assert_eq!(path.project().as_str(), "webshop");
        assert_eq!(path.name(), "checkout-flow");
        assert_eq!(path.to_string(), "projects/webshop/notes/checkout-flow.md");

        let spaced: NotePath = "projects/webshop/notes/Auth Middleware.md".parse().unwrap();
        assert_eq!(spaced.name(), "Auth Middleware");

        for invalid in [
            "",
            "webshop/notes/a.md",
            "projects/webshop/a.md",
            "projects/webshop/notes/a.txt",
            "projects/webshop/notes/.md",
            "projects/webshop/notes/.hidden.md",
            "projects/webshop/notes/sub/a.md",
            "projects/Web/notes/a.md",
            "projects\\webshop\\notes\\a.md",
        ] {
            assert!(invalid.parse::<NotePath>().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn error_names_value_and_rule() {
        let err = "K7F3".parse::<BlockId>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid block id \"K7F3\", expected 4 characters from a-z and 0-9"
        );
    }
}
