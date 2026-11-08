//! What was chosen in the search, so that it is listed higher next time.
//! Each choice counts less as time passes, so what is no longer used drops
//! back down, while what is used regularly stays on top.

use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::config;

const KEY: &str = "search-usage";
/// After this many seconds, two weeks, a choice counts half.
const HALF_LIFE: f64 = 14.0 * 24.0 * 60.0 * 60.0;
/// Rows scoring less are forgotten, one chosen once after about seven weeks.
const FORGOTTEN: f64 = 0.1;

/// The score of each row chosen and when it was last chosen, in seconds
/// since 1970, by the key of the row.
#[derive(Debug, Default)]
pub struct Usage(HashMap<String, (f64, i64)>);

impl Usage {
    /// The usage stored in the settings of this device.
    pub fn load() -> Self {
        Self(settings().get(KEY))
    }

    pub fn save(&self) {
        if let Err(err) = settings().set(KEY, &self.0) {
            glib::g_warning!("bitlog", "Cannot save what was chosen in the search: {err}");
        }
    }

    /// How much `key` was chosen as of `now`: each choice adds one, which
    /// halves every two weeks.
    pub fn score(&self, key: &str, now: i64) -> f64 {
        self.0
            .get(key)
            .map_or(0.0, |&(score, chosen)| faded(score, chosen, now))
    }

    /// Counts a choice of `key` at `now` and forgets the rows that have
    /// faded away.
    pub fn record(&mut self, key: &str, now: i64) {
        let score = self.score(key, now) + 1.0;
        self.0.insert(key.to_owned(), (score, now));
        self.0
            .retain(|_, &mut (score, chosen)| faded(score, chosen, now) >= FORGOTTEN);
    }
}

fn faded(score: f64, chosen: i64, now: i64) -> f64 {
    score * 0.5_f64.powf((now - chosen) as f64 / HALF_LIFE)
}

fn settings() -> gio::Settings {
    gio::Settings::new(config::app_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 60 * 60;

    #[test]
    fn choices_add_up_and_fade() {
        let mut usage = Usage::default();
        assert_eq!(usage.score("a", 0), 0.0);
        usage.record("a", 0);
        usage.record("a", 0);
        assert_eq!(usage.score("a", 0), 2.0);
        assert!((usage.score("a", 14 * DAY) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn regular_use_beats_old_habits() {
        let mut usage = Usage::default();
        for _ in 0..10 {
            usage.record("old", 0);
        }
        for week in 0..4 {
            usage.record("regular", 60 * DAY + week * 7 * DAY);
        }
        let now = 90 * DAY;
        assert!(usage.score("regular", now) > usage.score("old", now));
    }

    #[test]
    fn faded_rows_are_forgotten() {
        let mut usage = Usage::default();
        usage.record("once", 0);
        usage.record("other", 40 * DAY);
        assert!(usage.0.contains_key("once"));
        usage.record("other", 50 * DAY);
        assert!(!usage.0.contains_key("once"));
    }
}
