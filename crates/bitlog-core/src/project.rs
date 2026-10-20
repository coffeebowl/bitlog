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

#[derive(Debug, Clone)]
pub struct Project {
    /// The name of the project folder.
    pub slug: ProjectSlug,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    status: ProjectStatus,
    /// The status as the file has it if this version does not know it, kept
    /// in the file until another one is set.
    unknown_status: Option<String>,
    category: ProjectCategory,
    /// The category as the file has it if this version does not know it,
    /// kept in the file until another one is set.
    unknown_category: Option<String>,
    pub created: Option<NaiveDate>,
    /// The file as read, so that writing keeps comments, formatting and
    /// fields this version does not know.
    document: DocumentMut,
    /// Of the file as read, `None` for new projects.
    hash: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProjectStatus {
    #[default]
    Active,
    Paused,
    Archived,
}

impl ProjectStatus {
    pub const ALL: [Self; 3] = [Self::Active, Self::Paused, Self::Archived];

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
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == text)
            .ok_or_else(|| EditError::InvalidStatus(text.to_owned()))
    }
}

/// Whether the time of a project counts as working time and whether it is
/// work, one value for each of the four combinations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProjectCategory {
    /// Work that counts as working time.
    #[default]
    Work,
    /// Work that does not count, such as a side project.
    Private,
    /// Counts as working time without being work, such as a business trip.
    Duty,
    /// Neither, time off.
    Break,
}

impl ProjectCategory {
    pub const ALL: [Self; 4] = [Self::Work, Self::Private, Self::Duty, Self::Break];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Private => "private",
            Self::Duty => "duty",
            Self::Break => "break",
        }
    }

    pub fn counts_as_working_time(self) -> bool {
        matches!(self, Self::Work | Self::Duty)
    }

    /// Whether blocks of such projects are work, which block categories
    /// divide further.
    pub fn is_work(self) -> bool {
        matches!(self, Self::Work | Self::Private)
    }
}

impl FromStr for ProjectCategory {
    type Err = EditError;

    fn from_str(text: &str) -> Result<Self, EditError> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == text)
            .ok_or_else(|| EditError::InvalidProjectCategory(text.to_owned()))
    }
}

#[derive(Deserialize)]
struct ProjectFile {
    format: u32,
    name: Option<String>,
    color: Option<String>,
    status: Option<String>,
    category: Option<String>,
    #[serde(default, deserialize_with = "local_date")]
    created: Option<NaiveDate>,
}

/// The value `text` names, or the default and `text` itself if this version
/// does not know it, so that a typo or a value of a newer version does not
/// keep the vault from opening.
fn known_or_default<T: FromStr + Default>(text: Option<String>) -> (T, Option<String>) {
    match text {
        None => (T::default(), None),
        Some(text) => match text.parse() {
            Ok(value) => (value, None),
            Err(_) => (T::default(), Some(text)),
        },
    }
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
            status: ProjectStatus::default(),
            unknown_status: None,
            category: ProjectCategory::default(),
            unknown_category: None,
            created: Some(created),
            document: DocumentMut::new(),
            hash: None,
        }
    }

    /// The projects every new vault starts with, one of each category.
    pub fn defaults(created: NaiveDate) -> Vec<Self> {
        [
            ("company", "Your Company", "#3584e4", ProjectCategory::Work),
            (
                "side-project",
                "Your Side Project",
                "#33d17a",
                ProjectCategory::Private,
            ),
            (
                "business-trips",
                "Business Trips",
                "#2190a4",
                ProjectCategory::Duty,
            ),
            ("pause", "Break", "#9a9996", ProjectCategory::Break),
        ]
        .into_iter()
        .map(|(slug, name, color, category)| {
            let slug = slug.parse().expect("default slugs are valid");
            Self {
                color: color.to_owned(),
                category,
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

    /// Its status, `active` if the file has one this version does not know.
    pub fn status(&self) -> ProjectStatus {
        self.status
    }

    /// The status as the file has it, if this version does not know it.
    pub fn unknown_status(&self) -> Option<&str> {
        self.unknown_status.as_deref()
    }

    pub fn set_status(&mut self, status: ProjectStatus) {
        self.status = status;
        self.unknown_status = None;
    }

    /// Its category, `work` if the file has one this version does not know.
    pub fn category(&self) -> ProjectCategory {
        self.category
    }

    /// The category as the file has it, if this version does not know it.
    pub fn unknown_category(&self) -> Option<&str> {
        self.unknown_category.as_deref()
    }

    pub fn set_category(&mut self, category: ProjectCategory) {
        self.category = category;
        self.unknown_category = None;
    }

    /// The category of the project `slug`, looked up in `projects`. Blocks
    /// of projects missing there are work, as hand-edited days or removed
    /// projects leave them.
    pub(crate) fn category_in(projects: &[Project], slug: &ProjectSlug) -> ProjectCategory {
        projects
            .iter()
            .find(|project| project.slug == *slug)
            .map_or_else(ProjectCategory::default, Project::category)
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
        let (status, unknown_status) = known_or_default(file.status);
        let (category, unknown_category) = known_or_default(file.category);

        Ok(Self {
            name: file.name.unwrap_or_else(|| slug.to_string()),
            slug,
            color,
            status,
            unknown_status,
            category,
            unknown_category,
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
        if self.unknown_status.is_none() {
            set(table, "status", self.status.as_str().into());
        }
        if self.unknown_category.is_none() {
            set(table, "category", self.category.as_str().into());
        }
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
        assert_eq!(infra.status(), ProjectStatus::Active);
        assert_eq!(infra.category(), ProjectCategory::Work);
        assert_eq!(infra.created, Some(date(2026, 4, 15)));

        let breaks: Vec<&str> = projects
            .iter()
            .filter(|p| p.category() == ProjectCategory::Break)
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
        project.set_category(ProjectCategory::Private);
        assert_eq!(
            project.to_toml(),
            "format = 1\nname = \"Infrastructure\" # short\n# Billed separately.\ncategory = \"private\"\nhomepage = \"x\"\ncolor = \"#3584e4\"\nstatus = \"active\"\n"
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
    fn default_projects_have_one_of_each_category() {
        let defaults = Project::defaults(date(2026, 3, 1));
        let categories: Vec<_> = defaults.iter().map(Project::category).collect();
        assert_eq!(categories, ProjectCategory::ALL);
        let pause = defaults.last().unwrap();
        let sample = Project::load(&sample_vault(), pause.slug.clone()).unwrap();
        assert_eq!(pause.to_toml(), sample.to_toml());
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
    fn categories_from_text() {
        for category in ProjectCategory::ALL {
            assert_eq!(category.as_str().parse(), Ok(category));
        }
        assert_eq!(
            "overhead".parse::<ProjectCategory>(),
            Err(EditError::InvalidProjectCategory("overhead".to_owned()))
        );
    }

    #[test]
    fn unknown_statuses_count_as_active_and_are_kept() {
        let mut project = Project::parse(slug("misc"), "format = 1\nstatus = \"done\"\n").unwrap();
        assert_eq!(project.status(), ProjectStatus::Active);
        assert_eq!(project.unknown_status(), Some("done"));
        assert!(project.to_toml().contains("status = \"done\"\n"));
        project.set_status(ProjectStatus::Archived);
        assert_eq!(project.unknown_status(), None);
        assert!(project.to_toml().contains("status = \"archived\"\n"));
    }

    #[test]
    fn unknown_categories_count_as_work_and_are_kept() {
        let text = "format = 1\ncategory = \"overhead\" # old\n";
        let mut project = Project::parse(slug("misc"), text).unwrap();
        assert_eq!(project.category(), ProjectCategory::Work);
        assert_eq!(project.unknown_category(), Some("overhead"));
        assert!(
            project
                .to_toml()
                .contains("category = \"overhead\" # old\n"),
            "{}",
            project.to_toml()
        );
        project.set_category(ProjectCategory::Work);
        assert_eq!(project.unknown_category(), None);
        assert!(project.to_toml().contains("category = \"work\" # old\n"));
    }

    #[test]
    fn categories_answer_both_questions() {
        let answers: Vec<_> = ProjectCategory::ALL
            .into_iter()
            .map(|category| (category.counts_as_working_time(), category.is_work()))
            .collect();
        assert_eq!(
            answers,
            [(true, true), (false, true), (true, false), (false, false)]
        );
    }

    #[test]
    fn defaults() {
        let project = Project::parse(slug("misc"), "format = 1").unwrap();
        assert_eq!(project.name, "misc");
        assert_eq!(project.color, DEFAULT_COLOR);
        assert_eq!(project.status(), ProjectStatus::Active);
        assert_eq!(project.category(), ProjectCategory::Work);
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
