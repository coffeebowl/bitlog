//! Projects, `projects/<slug>/project.toml`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Deserializer};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::ProjectSlug;
use crate::error::ReadError;
use crate::file::{content_hash, parse_text, read_file};

/// The only format version this code knows.
const FORMAT: u32 = 1;

const DEFAULT_COLOR: &str = "#3584e4";
const DEFAULT_CATEGORY: &str = "work";

/// The category of projects whose blocks are breaks, not working time.
const BREAK_CATEGORY: &str = "break";

#[derive(Debug, Clone)]
pub struct Project {
    /// The name of the project folder.
    pub slug: ProjectSlug,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    pub status: ProjectStatus,
    /// Free text, only `break` has a meaning.
    pub category: String,
    /// Listed first when picking a project for a block.
    pub pinned: bool,
    pub created: Option<NaiveDate>,
    /// The file as read, so that writing keeps comments, formatting and
    /// fields this version does not know.
    document: DocumentMut,
    /// Of the file as read, `None` for new projects.
    hash: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Paused,
    Archived,
}

impl ProjectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Archived => "archived",
        }
    }
}

#[derive(Deserialize)]
struct ProjectFile {
    format: u32,
    name: Option<String>,
    color: Option<String>,
    status: Option<ProjectStatus>,
    category: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default, deserialize_with = "local_date")]
    created: Option<NaiveDate>,
}

/// Reads a TOML local date such as `2026-03-01`.
fn local_date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<NaiveDate>, D::Error> {
    use serde::de::Error;

    let value = toml::value::Datetime::deserialize(deserializer)?;
    match value {
        toml::value::Datetime {
            date: Some(date),
            time: None,
            offset: None,
        } => NaiveDate::from_ymd_opt(date.year.into(), date.month.into(), date.day.into())
            .map(Some)
            .ok_or_else(|| D::Error::custom(format!("invalid date {value}"))),
        _ => Err(D::Error::custom(format!(
            "expected a date like 2026-03-01, found {value}"
        ))),
    }
}

fn is_color(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl Project {
    /// A new project with default values for everything but its name.
    pub fn new(slug: ProjectSlug, name: &str, created: NaiveDate) -> Self {
        Self {
            slug,
            name: name.to_owned(),
            color: DEFAULT_COLOR.to_owned(),
            status: ProjectStatus::Active,
            category: DEFAULT_CATEGORY.to_owned(),
            pinned: false,
            created: Some(created),
            document: DocumentMut::new(),
            hash: None,
        }
    }

    /// The projects every new vault starts with.
    pub fn defaults(created: NaiveDate) -> Vec<Self> {
        [
            ("pause", "Break", "#9a9996", BREAK_CATEGORY),
            ("meetings", "Meetings", "#f6d32d", "overhead"),
            ("filler", "Filler", "#c061cb", "overhead"),
        ]
        .into_iter()
        .map(|(slug, name, color, category)| {
            let slug = slug.parse().expect("default slugs are valid");
            Self {
                color: color.to_owned(),
                category: category.to_owned(),
                pinned: true,
                ..Self::new(slug, name, created)
            }
        })
        .collect()
    }

    /// Whether blocks of this project are breaks.
    pub fn is_break(&self) -> bool {
        self.category == BREAK_CATEGORY
    }

    /// Where the file of the project `slug` lives in `vault`.
    pub fn path(vault: &Path, slug: &ProjectSlug) -> PathBuf {
        vault
            .join("projects")
            .join(slug.as_str())
            .join("project.toml")
    }

    pub fn load(vault: &Path, slug: ProjectSlug) -> Result<Self, ReadError> {
        read_file(&Self::path(vault, &slug), |text| Self::parse(slug, text))
    }

    /// Reads the content `text` of the file of the project `slug`.
    pub(crate) fn read(vault: &Path, slug: ProjectSlug, text: &str) -> Result<Self, ReadError> {
        parse_text(&Self::path(vault, &slug), text, |text| {
            Self::parse(slug, text)
        })
    }

    /// Whether this project was read from a file with the content `text`.
    pub(crate) fn is_read_from(&self, text: &str) -> bool {
        self.hash == Some(content_hash(text))
    }

    /// Loads all projects of `vault`, sorted by slug.
    ///
    /// Only folders below `projects/` whose name is a valid slug and that
    /// contain a `project.toml` are projects. Everything else is ignored.
    pub fn load_all(vault: &Path) -> Result<Vec<Self>, ReadError> {
        let folder = vault.join("projects");
        let io_error = |source| ReadError::Io {
            path: folder.clone(),
            source,
        };
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error(err)),
        };
        let mut projects = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let Some(slug) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<ProjectSlug>().ok())
            else {
                continue;
            };
            if Self::path(vault, &slug).is_file() {
                projects.push(Self::load(vault, slug)?);
            }
        }
        projects.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(projects)
    }

    fn parse(slug: ProjectSlug, text: &str) -> Result<Self, String> {
        let file: ProjectFile = toml::from_str(text).map_err(|err| err.to_string())?;
        // Read a second time, keeping comments and formatting for writing.
        let document: DocumentMut = text
            .parse()
            .map_err(|err: toml_edit::TomlError| err.to_string())?;

        if file.format != FORMAT {
            return Err(format!("unsupported format version {}", file.format));
        }
        let color = file.color.unwrap_or_else(|| DEFAULT_COLOR.to_owned());
        if !is_color(&color) {
            return Err(format!("color must look like \"#3584e4\", found {color:?}"));
        }

        Ok(Self {
            name: file.name.unwrap_or_else(|| slug.to_string()),
            slug,
            color,
            status: file.status.unwrap_or(ProjectStatus::Active),
            category: file.category.unwrap_or_else(|| DEFAULT_CATEGORY.to_owned()),
            pinned: file.pinned,
            created: file.created,
            document,
            hash: Some(content_hash(text)),
        })
    }

    /// The content of `project.toml`. Comments, formatting and unknown fields
    /// of the file this project was read from are kept, unchanged values are
    /// written exactly as they were.
    pub fn to_toml(&self) -> String {
        let mut document = self.document.clone();
        let table = document.as_table_mut();
        set(table, "format", i64::from(FORMAT).into());
        set(table, "name", self.name.as_str().into());
        set(table, "color", self.color.as_str().into());
        set(table, "status", self.status.as_str().into());
        set(table, "category", self.category.as_str().into());
        set(table, "pinned", self.pinned.into());
        match self.created {
            Some(date) => set(table, "created", toml_date(date).into()),
            None => {
                table.remove("created");
            }
        }
        document.to_string()
    }
}

fn toml_date(date: NaiveDate) -> toml_edit::Datetime {
    toml_edit::Datetime {
        date: Some(toml_edit::Date {
            year: u16::try_from(date.year()).expect("project dates lie in years 0 to 9999"),
            month: u8::try_from(date.month()).expect("months fit into u8"),
            day: u8::try_from(date.day()).expect("days fit into u8"),
        }),
        time: None,
        offset: None,
    }
}

/// Sets `key` to `value`, leaving it untouched if it already has this value
/// and keeping the comments around it otherwise.
fn set(table: &mut Table, key: &str, value: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) if same_value(old, &value) => {}
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(value));
        }
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Datetime(a), Value::Datetime(b)) => a.value() == b.value(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vault() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault")
    }

    fn slug(value: &str) -> ProjectSlug {
        value.parse().unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn sample_projects() {
        let projects = Project::load_all(&sample_vault()).unwrap();
        let slugs: Vec<&str> = projects.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["filler", "infra", "meetings", "pause", "webshop"]);

        let infra = &projects[1];
        assert_eq!(infra.name, "Infrastructure");
        assert_eq!(infra.color, "#2ec27e");
        assert_eq!(infra.status, ProjectStatus::Active);
        assert_eq!(infra.category, "work");
        assert!(!infra.pinned);
        assert_eq!(infra.created, Some(date(2026, 4, 15)));

        let breaks: Vec<&str> = projects
            .iter()
            .filter(|p| p.is_break())
            .map(|p| p.slug.as_str())
            .collect();
        assert_eq!(breaks, ["pause"]);
    }

    #[test]
    fn unchanged_files_are_written_byte_identical() {
        for project in Project::load_all(&sample_vault()).unwrap() {
            let text = fs::read_to_string(Project::path(&sample_vault(), &project.slug)).unwrap();
            assert_eq!(project.to_toml(), text, "{}", project.slug);
        }
    }

    #[test]
    fn writing_keeps_comments_and_unknown_fields() {
        let mut project = Project::parse(
            slug("infra"),
            "format = 1\nname = 'Infra' # short\n# Billed separately.\ncategory = \"work\"\nhomepage = \"x\"\n",
        )
        .unwrap();
        project.name = "Infrastructure".to_owned();
        project.category = "ops".to_owned();
        project.pinned = true;
        assert_eq!(
            project.to_toml(),
            "format = 1\nname = \"Infrastructure\" # short\n# Billed separately.\ncategory = \"ops\"\nhomepage = \"x\"\ncolor = \"#3584e4\"\nstatus = \"active\"\npinned = true\n"
        );
    }

    #[test]
    fn new_project_roundtrip() {
        let project = Project::new(slug("webshop"), "Webshop", date(2026, 3, 1));
        let text = project.to_toml();
        assert_eq!(
            text,
            "format = 1\nname = \"Webshop\"\ncolor = \"#3584e4\"\nstatus = \"active\"\ncategory = \"work\"\npinned = false\ncreated = 2026-03-01\n"
        );
        let read = Project::parse(slug("webshop"), &text).unwrap();
        assert_eq!(read.to_toml(), text);
        assert_eq!(read.created, Some(date(2026, 3, 1)));
    }

    #[test]
    fn default_projects_match_sample_vault() {
        for default in Project::defaults(date(2026, 3, 1)) {
            let sample = Project::load(&sample_vault(), default.slug.clone()).unwrap();
            assert_eq!(default.to_toml(), sample.to_toml(), "{}", default.slug);
        }
    }

    #[test]
    fn defaults() {
        let project = Project::parse(slug("misc"), "format = 1").unwrap();
        assert_eq!(project.name, "misc");
        assert_eq!(project.color, DEFAULT_COLOR);
        assert_eq!(project.status, ProjectStatus::Active);
        assert_eq!(project.category, "work");
        assert!(!project.pinned);
        assert_eq!(project.created, None);
        assert!(!project.to_toml().contains("created"));
    }

    #[test]
    fn invalid() {
        for text in [
            "",
            "format = 2",
            "format = 1\ncolor = \"blue\"",
            "format = 1\ncolor = \"#12345\"",
            "format = 1\nstatus = \"done\"",
            "format = 1\npinned = \"yes\"",
            "format = 1\ncreated = \"2026-03-01\"",
            "format = 1\ncreated = 2026-03-01T10:00:00",
        ] {
            assert!(Project::parse(slug("a"), text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn missing_projects_folder() {
        assert!(
            Project::load_all(Path::new("/nonexistent"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn error_names_file() {
        let err = Project::load(Path::new("/nonexistent"), slug("a")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("cannot read /nonexistent/projects/a/project.toml")
        );
    }
}
