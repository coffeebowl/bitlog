//! How times and shares are written wherever the language does not matter.

use chrono::TimeDelta;

use crate::time_at_minute;

/// A span of a day as [`Block::span`](crate::Block::span) gives it, as in
/// `09:00–10:30`, with `+1` behind an end on the next day.
pub fn format_span((start, end): (u32, u32)) -> String {
    let time = |minute| time_at_minute(minute).format("%H:%M");
    let next_day = if end >= 24 * 60 { "+1" } else { "" };
    format!("{}–{}{next_day}", time(start), time(end))
}

/// A duration as hours and minutes where there is little room, as in `7:45`.
pub fn format_short_duration(duration: TimeDelta) -> String {
    let minutes = duration.num_minutes();
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// `part` as a share of `whole` in percent, rounded to a whole number. No
/// time is no share of no time.
pub fn percent(part: TimeDelta, whole: TimeDelta) -> i64 {
    let whole = whole.num_minutes().max(1);
    (part.num_minutes() * 100 + whole / 2) / whole
}

/// `text` with a capital first letter, as kinds of day and categories are
/// usually written in lowercase.
pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_past_midnight_are_marked() {
        assert_eq!(format_span((9 * 60, 10 * 60 + 30)), "09:00–10:30");
        assert_eq!(format_span((22 * 60 + 30, 24 * 60 + 30)), "22:30–00:30+1");
        assert_eq!(format_span((22 * 60, 24 * 60)), "22:00–00:00+1");
    }

    #[test]
    fn short_durations() {
        assert_eq!(format_short_duration(TimeDelta::minutes(465)), "7:45");
        assert_eq!(format_short_duration(TimeDelta::minutes(5)), "0:05");
    }

    #[test]
    fn percent_is_rounded() {
        let minutes = TimeDelta::minutes;
        assert_eq!(percent(minutes(1), minutes(3)), 33);
        assert_eq!(percent(minutes(2), minutes(3)), 67);
        assert_eq!(percent(minutes(0), minutes(0)), 0);
    }

    #[test]
    fn capitalized() {
        assert_eq!(capitalize("work"), "Work");
        assert_eq!(capitalize(""), "");
    }
}
