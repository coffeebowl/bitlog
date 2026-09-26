//! The vault configuration, `knotbook.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveTime, Weekday};
use serde::{Deserialize, Deserializer};

use crate::LocationKey;
use crate::error::{ReadError, read_file};

/// The only format version this code knows.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WeekConfig {
    pub first_day: Weekday,
    pub workdays: Vec<Weekday>,
    /// Only shown for orientation, nothing depends on it.
    pub target_hours: f64,
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

fn default_name() -> String {
    "Knotbook".to_owned()
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

/// Reads a TOML local time such as `07:00:00`.
fn local_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveTime, D::Error> {
    use serde::de::Error;

    let value = toml::value::Datetime::deserialize(deserializer)?;
    match value {
        toml::value::Datetime {
            date: None,
            time: Some(time),
            offset: None,
        } => NaiveTime::from_hms_opt(
            time.hour.into(),
            time.minute.into(),
            time.second.unwrap_or(0).into(),
        )
        .ok_or_else(|| D::Error::custom(format!("invalid time {value}"))),
        _ => Err(D::Error::custom(format!(
            "expected a time of day like 07:00:00, found {value}"
        ))),
    }
}

impl VaultConfig {
    pub fn load(path: &Path) -> Result<Self, ReadError> {
        read_file(path, Self::parse)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(text).map_err(|err| err.to_string())?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("unsupported format version {}", self.format));
        }
        if self.week.target_hours < 0.0 {
            return Err("week.target_hours must not be negative".to_owned());
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
        Ok(())
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
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault/knotbook.toml");
        let config = VaultConfig::load(&path).unwrap();

        use Weekday::*;
        assert_eq!(config.name, "Sample Knotbook");
        assert_eq!(config.week.first_day, Mon);
        assert_eq!(config.week.workdays, [Mon, Tue, Wed, Thu]);
        assert_eq!(config.week.target_hours, 32.0);
        assert_eq!(config.grid.slot_minutes, 15);
        assert_eq!(config.grid.day_start, time(7, 0));
        assert_eq!(config.grid.day_end, time(19, 0));
        assert_eq!(config.locations.len(), 3);
        let office: LocationKey = "office".parse().unwrap();
        assert_eq!(config.locations[&office], "Office");
        assert_eq!(config.defaults.location, Some("remote".parse().unwrap()));
        assert_eq!(
            config.defaults.note_template,
            Some(PathBuf::from("templates/note.md"))
        );
    }

    #[test]
    fn defaults() {
        let config = VaultConfig::parse("format = 1").unwrap();
        assert_eq!(config.name, "Knotbook");
        assert_eq!(config.week, WeekConfig::default());
        assert_eq!(config.grid, GridConfig::default());
        assert!(config.locations.is_empty());
        assert_eq!(config.defaults, DefaultsConfig::default());
    }

    #[test]
    fn tolerant_reading() {
        let config = VaultConfig::parse(
            "format = 1\nunknown = true\n[week]\nfirst_day = \"Sunday\"\n[grid]\nday_start = 06:30\n",
        )
        .unwrap();
        assert_eq!(config.week.first_day, Weekday::Sun);
        assert_eq!(config.grid.day_start, time(6, 30));
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
            "format = 1\n[locations]\nHome = \"Home\"",
            "format = 1\n[defaults]\nlocation = \"office\"",
        ] {
            assert!(VaultConfig::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn error_names_file() {
        let err = VaultConfig::load(Path::new("/nonexistent/knotbook.toml")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("cannot read /nonexistent/knotbook.toml")
        );
    }
}
