//! The vault configuration, `bitlog.toml`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use chrono::{NaiveTime, Weekday};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::error::ReadError;
use crate::file::{FORMAT, check_format, content_hash, parse_text, parse_toml, read_file};
use crate::toml_values::{local_time, set, toml_time};
use crate::{LocationKey, ProjectSlug};

#[derive(Debug, Clone, Deserialize)]
pub struct VaultConfig {
    pub format: u32,
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub week: WeekConfig,
    #[serde(default)]
    pub grid: GridConfig,
    /// Display names by key.
    #[serde(default)]
    pub locations: BTreeMap<LocationKey, String>,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub projects: ProjectsConfig,
    /// The file as read, keeping comments and formatting for writing.
    #[serde(skip)]
    document: DocumentMut,
    /// The hash of the file as read, `None` for a configuration never read.
    #[serde(skip)]
    hash: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WeekConfig {
    pub first_day: Weekday,
    /// The days `target_hours` are shared among, as read: in any order,
    /// possibly repeated.
    pub workdays: Vec<Weekday>,
    /// Only shown for orientation, nothing depends on it.
    pub target_hours: f64,
}

impl WeekConfig {
    /// The workdays in the order of the week from Monday, each once.
    pub fn workdays(&self) -> Vec<Weekday> {
        weekdays_listed(&self.workdays)
    }

    /// The share of `target_hours` that falls on `weekday`: an even part on
    /// workdays, nothing on the other days.
    pub fn target_hours_on(&self, weekday: Weekday) -> f64 {
        if self.workdays.contains(&weekday) {
            self.target_hours / self.workdays().len() as f64
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct GridConfig {
    pub slot_minutes: u32,
    /// Start of the range the day view shows.
    #[serde(deserialize_with = "local_time")]
    pub day_start: NaiveTime,
    /// End of the range the day view shows.
    #[serde(deserialize_with = "local_time")]
    pub day_end: NaiveTime,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub location: Option<LocationKey>,
    /// Path relative to the vault.
    pub note_template: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProjectsConfig {
    /// The order in which projects are listed. Projects missing here come
    /// after the others, by name; slugs of projects the vault lacks are
    /// ignored.
    pub order: Vec<ProjectSlug>,
}

fn default_name() -> String {
    "BitLog".to_owned()
}

impl Default for WeekConfig {
    fn default() -> Self {
        use Weekday::*;
        Self {
            first_day: Mon,
            workdays: vec![Mon, Tue, Wed, Thu, Fri],
            target_hours: 40.0,
        }
    }
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            slot_minutes: 15,
            day_start: NaiveTime::from_hms_opt(7, 0, 0).expect("valid time"),
            day_end: NaiveTime::from_hms_opt(19, 0, 0).expect("valid time"),
        }
    }
}

impl VaultConfig {
    pub(crate) fn load(path: &Path) -> Result<Self, ReadError> {
        read_file(path, Self::parse)
    }

    /// Reads the content `text` of the file `path`.
    pub(crate) fn read(path: &Path, text: &str) -> Result<Self, ReadError> {
        parse_text(path, text, Self::parse)
    }

    /// Whether this configuration was read from a file with the content `text`.
    pub(crate) fn is_read_from(&self, text: &str) -> bool {
        self.hash == Some(content_hash(text))
    }

    /// The display name of the location `key`, or the key itself if
    /// `[locations]` lacks it, as in hand-edited day files.
    pub fn location_name<'a>(&'a self, key: &'a LocationKey) -> &'a str {
        self.locations.get(key).map_or(key.as_str(), String::as_str)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let (mut config, document, hash): (Self, _, _) = parse_toml(text)?;
        config.validate()?;
        config.document = document;
        config.hash = Some(hash);
        Ok(config)
    }

    /// The content of `bitlog.toml`, or why these settings are invalid.
    ///
    /// Comments, formatting and unknown fields of the file this configuration
    /// was read from are kept, unchanged values are written exactly as they
    /// were. Keys the file lacks are only added for values other than their
    /// default, so that a short file stays short.
    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        let (week, grid) = (WeekConfig::default(), GridConfig::default());
        let mut document = self.document.clone();
        let doc = &mut document;
        set_key(doc, None, "format", i64::from(FORMAT).into(), false);
        set_key(
            doc,
            None,
            "name",
            self.name.as_str().into(),
            self.name == default_name(),
        );
        set_key(
            doc,
            Some("week"),
            "first_day",
            weekday_name(self.week.first_day).into(),
            self.week.first_day == week.first_day,
        );
        set_key(
            doc,
            Some("week"),
            "target_hours",
            self.week.target_hours.into(),
            self.week.target_hours == week.target_hours,
        );
        // Only written when other days are meant, so that the file keeps its
        // spelling and order of the days.
        let written = doc
            .get("week")
            .and_then(|week| week.get("workdays"))
            .and_then(Item::as_array)
            .map_or_else(
                || week.workdays(),
                |days| {
                    let days: Vec<Weekday> = days
                        .iter()
                        .filter_map(|day| day.as_str()?.parse().ok())
                        .collect();
                    weekdays_listed(&days)
                },
            );
        if self.week.workdays() != written {
            let days: toml_edit::Array =
                self.week.workdays().into_iter().map(weekday_name).collect();
            set(section(doc, Some("week")), "workdays", days.into());
        }
        set_key(
            doc,
            Some("grid"),
            "slot_minutes",
            i64::from(self.grid.slot_minutes).into(),
            self.grid.slot_minutes == grid.slot_minutes,
        );
        set_key(
            doc,
            Some("grid"),
            "day_start",
            toml_time(self.grid.day_start).into(),
            self.grid.day_start == grid.day_start,
        );
        set_key(
            doc,
            Some("grid"),
            "day_end",
            toml_time(self.grid.day_end).into(),
            self.grid.day_end == grid.day_end,
        );
        let location = self
            .defaults
            .location
            .as_ref()
            .map(|key| key.as_str().into());
        set_optional(doc, "defaults", "location", location);
        let template = match &self.defaults.note_template {
            Some(path) => Some(
                path.to_str()
                    .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
                    .into(),
            ),
            None => None,
        };
        set_optional(doc, "defaults", "note_template", template);
        let order: toml_edit::Array = self
            .projects
            .order
            .iter()
            .map(ProjectSlug::as_str)
            .collect();
        set_key(
            doc,
            Some("projects"),
            "order",
            order.into(),
            self.projects.order.is_empty(),
        );
        Ok(document.to_string())
    }

    fn validate(&self) -> Result<(), String> {
        check_format(self.format)?;
        if self.week.target_hours < 0.0 {
            return Err("week.target_hours must not be negative".to_owned());
        }
        if self.week.workdays.is_empty() {
            return Err("week.workdays must name at least one day".to_owned());
        }
        // Slots have to line up with full hours, or the grid would drift.
        if self.grid.slot_minutes == 0 || 60 % self.grid.slot_minutes != 0 {
            return Err(format!(
                "grid.slot_minutes must divide 60, found {}",
                self.grid.slot_minutes
            ));
        }
        if self.grid.day_start >= self.grid.day_end {
            return Err("grid.day_start must be before grid.day_end".to_owned());
        }
        if let Some(location) = &self.defaults.location
            && !self.locations.contains_key(location)
        {
            return Err(format!(
                "defaults.location {location:?} is not listed in [locations]"
            ));
        }
        // New notes are made from the template, so it must not reach out of
        // the vault and copy other files into it.
        if let Some(template) = &self.defaults.note_template
            && !is_inside(template)
        {
            return Err(format!(
                "defaults.note_template {:?} must be a file in the vault, relative to it",
                template.display().to_string()
            ));
        }
        Ok(())
    }
}

/// Whether `path`, relative to the vault, names a file inside it.
fn is_inside(path: &Path) -> bool {
    path.file_name().is_some()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// The table `section` of `document`, or the document itself for `None`.
fn section<'a>(document: &'a mut DocumentMut, section: Option<&str>) -> &'a mut Table {
    let Some(section) = section else {
        return document.as_table_mut();
    };
    let item = document.entry(section).or_insert(toml_edit::table());
    if let Some(inline) = item.as_inline_table() {
        *item = Item::Table(inline.clone().into_table());
    }
    item.as_table_mut()
        .expect("the configuration sections are tables")
}

/// Sets `key` in `section` to `value`, unless the file lacks it and `value` is
/// its default anyway.
fn set_key(
    document: &mut DocumentMut,
    table: Option<&str>,
    key: &str,
    value: Value,
    is_default: bool,
) {
    let present = match table {
        None => document.contains_key(key),
        Some(table) => document
            .get(table)
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.contains_key(key)),
    };
    if present || !is_default {
        set(section(document, table), key, value);
    }
}

/// Sets `key` in `table` to `value`, or removes it if there is none.
fn set_optional(document: &mut DocumentMut, table: &str, key: &str, value: Option<Value>) {
    match value {
        Some(value) => set(section(document, Some(table)), key, value),
        None => {
            if let Some(table) = document.get_mut(table).and_then(Item::as_table_like_mut) {
                table.remove(key);
            }
        }
    }
}

/// The days of `days` in the order of the week from Monday, each once.
fn weekdays_listed(days: &[Weekday]) -> Vec<Weekday> {
    (0..7)
        .map(|n| Weekday::try_from(n).expect("seven weekdays"))
        .filter(|day| days.contains(day))
        .collect()
}

/// The name of `weekday` as `bitlog.toml` writes it, such as `mon`.
fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn sample_vault() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault/bitlog.toml");
        let config = VaultConfig::load(&path).unwrap();

        use Weekday::*;
        assert_eq!(config.name, "Sample BitLog");
        assert_eq!(config.week.first_day, Mon);
        assert_eq!(config.week.workdays, [Mon, Tue, Wed, Thu]);
        assert_eq!(config.week.target_hours, 32.0);
        assert_eq!(config.grid.slot_minutes, 15);
        assert_eq!(config.grid.day_start, time(7, 0));
        assert_eq!(config.grid.day_end, time(19, 0));
        assert_eq!(config.locations.len(), 3);
        let office: LocationKey = "office".parse().unwrap();
        assert_eq!(config.locations[&office], "Office");
        assert_eq!(config.location_name(&office), "Office");
        let moon: LocationKey = "moon".parse().unwrap();
        assert_eq!(config.location_name(&moon), "moon");
        assert_eq!(config.defaults.location, Some("remote".parse().unwrap()));
        assert_eq!(
            config.defaults.note_template,
            Some(PathBuf::from("templates/note.md"))
        );
        let order: Vec<&str> = config.projects.order.iter().map(|s| s.as_str()).collect();
        assert_eq!(order, ["webshop", "infra", "meetings", "filler"]);
    }

    #[test]
    fn defaults() {
        let config = VaultConfig::parse("format = 1").unwrap();
        assert_eq!(config.name, "BitLog");
        assert_eq!(config.week, WeekConfig::default());
        assert_eq!(config.grid, GridConfig::default());
        assert!(config.locations.is_empty());
        assert_eq!(config.defaults, DefaultsConfig::default());
        assert!(config.projects.order.is_empty());
    }

    #[test]
    fn tolerant_reading() {
        let config = VaultConfig::parse(
            "format = 1\nunknown = true\n[week]\nfirst_day = \"Sunday\"\n[grid]\nday_start = 06:30\n",
        )
        .unwrap();
        assert_eq!(config.week.first_day, Weekday::Sun);
        assert_eq!(config.grid.day_start, time(6, 30));
        let config =
            VaultConfig::parse("format = 1\n[defaults]\nnote_template = \"./note.md\"").unwrap();
        assert_eq!(
            config.defaults.note_template,
            Some(PathBuf::from("./note.md"))
        );
    }

    #[test]
    fn invalid() {
        for text in [
            "",
            "format = 2",
            "format = 1\n[grid]\nslot_minutes = 7",
            "format = 1\n[grid]\nslot_minutes = 0",
            "format = 1\n[grid]\nday_start = 19:00:00\nday_end = 07:00:00",
            "format = 1\n[grid]\nday_start = 2026-01-01T07:00:00",
            "format = 1\n[grid]\nday_start = \"07:00\"",
            "format = 1\n[week]\nfirst_day = \"someday\"",
            "format = 1\n[week]\ntarget_hours = -1.0",
            "format = 1\n[week]\nworkdays = []",
            "format = 1\n[locations]\nHome = \"Home\"",
            "format = 1\n[defaults]\nlocation = \"office\"",
            "format = 1\n[defaults]\nnote_template = \"/etc/passwd\"",
            "format = 1\n[defaults]\nnote_template = \"../note.md\"",
            "format = 1\n[defaults]\nnote_template = \"templates/../../note.md\"",
            "format = 1\n[defaults]\nnote_template = \"\"",
            "format = 1\n[projects]\norder = [\"Web Shop\"]",
        ] {
            assert!(VaultConfig::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn unchanged_files_are_written_byte_identical() {
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault/bitlog.toml");
        let init = include_str!("init/bitlog.toml");
        for text in [
            std::fs::read_to_string(sample).unwrap().as_str(),
            init,
            "format = 1\n",
        ] {
            let config = VaultConfig::parse(text).unwrap();
            assert_eq!(config.to_toml().unwrap(), text);
        }
    }

    #[test]
    fn writing_keeps_comments() {
        let mut config = VaultConfig::parse(
            "format = 1\n[grid]\nslot_minutes = 15 # quarter hours\nday_end = 19:00:00\n",
        )
        .unwrap();
        config.grid.slot_minutes = 30;
        config.grid.day_end = time(20, 30);
        config.week.target_hours = 40.0;
        assert_eq!(
            config.to_toml().unwrap(),
            "format = 1\n[grid]\nslot_minutes = 30 # quarter hours\nday_end = 20:30:00\n"
        );
    }

    #[test]
    fn writing_adds_only_changed_keys() {
        let mut config = VaultConfig::parse("format = 1\n").unwrap();
        config.name = "Work".to_owned();
        config.week.first_day = Weekday::Sun;
        config.grid.day_start = time(6, 30);
        config.defaults.note_template = Some(PathBuf::from("templates/note.md"));
        let text = config.to_toml().unwrap();
        assert_eq!(
            text,
            "format = 1\nname = \"Work\"\n\n[week]\nfirst_day = \"sun\"\n\n[grid]\nday_start = 06:30:00\n\n[defaults]\nnote_template = \"templates/note.md\"\n"
        );
        let read = VaultConfig::parse(&text).unwrap();
        assert_eq!(read.week, config.week);
        assert_eq!(read.grid, config.grid);
        assert_eq!(read.defaults, config.defaults);
    }

    #[test]
    fn target_hours_shared_among_workdays() {
        use Weekday::*;
        let config = VaultConfig::parse(
            "format = 1\n[week]\nworkdays = [\"thu\", \"Monday\", \"tue\", \"wed\", \"mon\"]\ntarget_hours = 32.0",
        )
        .unwrap();
        assert_eq!(config.week.workdays(), [Mon, Tue, Wed, Thu]);
        assert_eq!(config.week.target_hours_on(Mon), 8.0);
        assert_eq!(config.week.target_hours_on(Thu), 8.0);
        assert_eq!(config.week.target_hours_on(Fri), 0.0);
    }

    #[test]
    fn writing_workdays() {
        use Weekday::*;
        let text = "format = 1\n[week]\nworkdays = [\"Thursday\", \"mon\", \"tue\", \"wed\"]\n";
        let mut config = VaultConfig::parse(text).unwrap();
        config.week.workdays = vec![Mon, Tue, Wed, Thu];
        assert_eq!(config.to_toml().unwrap(), text);
        config.week.workdays = vec![Fri, Mon];
        let text = config.to_toml().unwrap();
        assert_eq!(text, "format = 1\n[week]\nworkdays = [\"mon\", \"fri\"]\n");

        let mut config = VaultConfig::parse("format = 1\n").unwrap();
        config.week.workdays = vec![Mon, Tue, Wed, Thu, Fri];
        assert_eq!(config.to_toml().unwrap(), "format = 1\n");
        config.week.workdays = vec![Sat, Sun];
        assert_eq!(
            config.to_toml().unwrap(),
            "format = 1\n\n[week]\nworkdays = [\"sat\", \"sun\"]\n"
        );
    }

    #[test]
    fn writing_the_project_order() {
        let mut config = VaultConfig::parse("format = 1\n").unwrap();
        config.projects.order = vec!["b".parse().unwrap(), "a".parse().unwrap()];
        let text = config.to_toml().unwrap();
        assert_eq!(text, "format = 1\n\n[projects]\norder = [\"b\", \"a\"]\n");
        assert_eq!(VaultConfig::parse(&text).unwrap().projects, config.projects);
    }

    #[test]
    fn writing_removes_unset_defaults() {
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault/bitlog.toml");
        let mut config = VaultConfig::load(&sample).unwrap();
        config.defaults = DefaultsConfig::default();
        let text = config.to_toml().unwrap();
        assert!(!text.contains("location ="), "{text}");
        assert!(!text.contains("note_template"), "{text}");
        assert!(text.contains("[defaults]"), "{text}");
    }

    #[test]
    fn writing_rejects_invalid_settings() {
        let mut config = VaultConfig::parse("format = 1").unwrap();
        config.grid.day_start = time(20, 0);
        assert!(config.to_toml().is_err());
    }

    #[test]
    fn error_names_file() {
        let err = VaultConfig::load(Path::new("/nonexistent/bitlog.toml")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("cannot read /nonexistent/bitlog.toml")
        );
    }
}
