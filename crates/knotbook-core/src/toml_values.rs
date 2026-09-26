//! Reading and writing values of the TOML files a vault holds.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Deserializer};
use toml_edit::{Item, Table, Value};

/// Reads a TOML local date such as `2026-03-01`.
pub(crate) fn local_date<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<NaiveDate>, D::Error> {
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

pub(crate) fn toml_date(date: NaiveDate) -> toml_edit::Datetime {
    toml_edit::Datetime {
        date: Some(toml_edit::Date {
            year: u16::try_from(date.year()).expect("vault dates lie in years 0 to 9999"),
            month: u8::try_from(date.month()).expect("months fit into u8"),
            day: u8::try_from(date.day()).expect("days fit into u8"),
        }),
        time: None,
        offset: None,
    }
}

/// Sets `key` to `value`, leaving it untouched if it already has this value
/// and keeping the comments around it otherwise.
pub(crate) fn set(table: &mut Table, key: &str, value: Value) {
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

/// Sets `key` to the date `date`, or removes it if there is none.
pub(crate) fn set_date(table: &mut Table, key: &str, date: Option<NaiveDate>) {
    match date {
        Some(date) => set(table, key, toml_date(date).into()),
        None => {
            table.remove(key);
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
