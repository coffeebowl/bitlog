//! Projects, `projects/<slug>/project.toml`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use serde::Deserialize;
use toml_edit::DocumentMut;

use crate::error::ReadError;
use crate::file::{
    FORMAT, check_format, content_hash, parse_text, parse_toml, read_file, read_folder,
};
use crate::toml_values::{local_date, set, set_date};
use crate::{EditError, ProjectSlug};

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

impl FromStr for ProjectStatus {
    type Err = EditError;

    fn from_str(text: &str) -> Result<Self, EditError> {
        [Self::Active, Self::Paused, Self::Archived]
            .into_iter()
            .find(|status| status.as_str() == text)
            .ok_or_else(|| EditError::InvalidStatus(text.to_owned()))
    }
}

#[derive(Deserialize)]
struct ProjectFile {
    format: u32,
    name: Option<String>,
    color: Option<String>,
    status: Option<ProjectStatus>,
    category: Option<String>,
    #[serde(default, deserialize_with = "local_date")]
    created: Option<NaiveDate>,
}

/// The folder of the project `slug` in `vault`.
pub(crate) fn project_folder(vault: &Path, slug: &ProjectSlug) -> PathBuf {
    vault.join("projects").join(slug.as_str())
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
                ..Self::new(slug, name, created)
            }
        })
        .collect()
    }

    /// Sets the color, `#` and six hex digits.
    pub fn set_color(&mut self, color: &str) -> Result<(), EditError> {
        if !is_color(color) {
            return Err(EditError::InvalidColor(color.to_owned()));
        }
        self.color = color.to_owned();
        Ok(())
    }

    /// Whether blocks of this project are breaks.
    pub fn is_break(&self) -> bool {
        self.category == BREAK_CATEGORY
    }

    /// Whether blocks of the project `slug` are breaks, looked up in
    /// `projects`. Blocks of projects missing there are work, as hand-edited
    /// days or removed projects leave them.
    pub(crate) fn is_break_in(projects: &[Project], slug: &ProjectSlug) -> bool {
        projects
            .iter()
            .any(|project| project.slug == *slug && project.is_break())
    }

    /// Where the file of the project `slug` lives in `vault`.
    pub fn path(vault: &Path, slug: &ProjectSlug) -> PathBuf {
        project_folder(vault, slug).join("project.toml")
    }

    fn load(vault: &Path, slug: ProjectSlug) -> Result<Self, ReadError> {
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
        let mut projects = Vec::new();
        for entry in read_folder(&vault.join("projects"))? {
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
        let (file, document, hash): (ProjectFile, _, _) = parse_toml(text)?;
        check_format(file.format)?;
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
            created: file.created,
            document,
            hash: Some(hash),
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
        set_date(table, "created", self.created);
        document.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

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
        assert_eq!(
            project.to_toml(),
            "format = 1\nname = \"Infrastructure\" # short\n# Billed separately.\ncategory = \"ops\"\nhomepage = \"x\"\ncolor = \"#3584e4\"\nstatus = \"active\"\n"
        );
    }

    #[test]
    fn new_project_roundtrip() {
        let project = Project::new(slug("webshop"), "Webshop", date(2026, 3, 1));
        let text = project.to_toml();
        assert_eq!(
            text,
            "format = 1\nname = \"Webshop\"\ncolor = \"#3584e4\"\nstatus = \"active\"\ncategory = \"work\"\ncreated = 2026-03-01\n"
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
    fn status_from_text() {
        assert_eq!("paused".parse(), Ok(ProjectStatus::Paused));
        assert_eq!(
            "done".parse::<ProjectStatus>(),
            Err(EditError::InvalidStatus("done".to_owned()))
        );
    }

    #[test]
    fn defaults() {
        let project = Project::parse(slug("misc"), "format = 1").unwrap();
        assert_eq!(project.name, "misc");
        assert_eq!(project.color, DEFAULT_COLOR);
        assert_eq!(project.status, ProjectStatus::Active);
        assert_eq!(project.category, "work");
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
