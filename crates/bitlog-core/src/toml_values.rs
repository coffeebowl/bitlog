//! Reading and writing values of the TOML files a vault holds.

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
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

/// Reads a TOML local time such as `07:00:00`.
pub(crate) fn local_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<NaiveTime, D::Error> {
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

pub(crate) fn toml_time(time: NaiveTime) -> toml_edit::Datetime {
    let part = |value: u32| u8::try_from(value).expect("time parts fit into u8");
    toml_edit::Datetime {
        date: None,
        time: Some(toml_edit::Time {
            hour: part(time.hour()),
            minute: part(time.minute()),
            second: Some(part(time.second())),
            nanosecond: None,
        }),
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

/// Sets `key` to `text`, or removes it if `text` is empty.
pub(crate) fn set_text(table: &mut Table, key: &str, text: &str) {
    if text.is_empty() {
        table.remove(key);
    } else {
        set(table, key, text.into());
    }
}

/// The texts of the table `key` in `table` with their labels, the keys, in
/// the order of the file. None if there is no such table.
pub(crate) fn labeled_texts(table: &Table, key: &str) -> Result<Vec<(String, String)>, String> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let labeled = item
        .as_table_like()
        .ok_or_else(|| format!("{key} must be a table"))?;
    labeled
        .iter()
        .map(|(label, item)| {
            item.as_str()
                .map(|text| (label.to_owned(), text.to_owned()))
                .ok_or_else(|| format!("{key}.{label} must be a string"))
        })
        .collect()
}

/// Makes `key` a table of `texts` with their labels as keys, in this order,
/// or removes it if there are none. The comments around texts that stay are
/// kept; an inline table becomes a table of its own.
pub(crate) fn set_labeled_texts(table: &mut Table, key: &str, texts: &[(String, String)]) {
    if texts.is_empty() {
        table.remove(key);
        return;
    }
    if !table.get(key).is_some_and(Item::is_table) {
        table.insert(key, Item::Table(Table::new()));
    }
    let labeled = table
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .expect("the table was just made");
    labeled.retain(|label, _| texts.iter().any(|(other, _)| other == label));
    for (label, text) in texts {
        set(labeled, label, text.as_str().into());
    }
    let position = |label: &str| texts.iter().position(|(other, _)| other == label);
    labeled.sort_values_by(|a, _, b, _| position(a.get()).cmp(&position(b.get())));
}

/// Whether `a` and `b` hold the same, however they are written.
pub(crate) fn same_item(a: &Item, b: &Item) -> bool {
    match (a.as_value(), b.as_value()) {
        (Some(a), Some(b)) => same_value(a, b) || bare(a) == bare(b),
        _ => a.to_string().trim() == b.to_string().trim(),
    }
}

/// `value` as written, without comments and spaces around it.
fn bare(value: &Value) -> String {
    let mut value = value.clone();
    value.decor_mut().clear();
    value.to_string()
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Float(a), Value::Float(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Datetime(a), Value::Datetime(b)) => a.value() == b.value(),
        _ => false,
    }
}
