//! Weeks, months and years, as the calendar, reports and statistics show them.

use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Week,
    Month,
    Year,
}

impl Period {
    /// The first and last day of the period that holds `date`. Weeks start
    /// on `first_day`.
    pub fn range(self, date: NaiveDate, first_day: Weekday) -> (NaiveDate, NaiveDate) {
        let first = match self {
            Self::Week => week_start(date, first_day),
            Self::Month => date.with_day(1).expect("every month has a first day"),
            Self::Year => date.with_ordinal(1).expect("every year has a first day"),
        };
        let next = match self {
            Self::Week => first + Days::new(7),
            Self::Month => first + Months::new(1),
            Self::Year => first + Months::new(12),
        };
        (first, next.pred_opt().expect("the day before exists"))
    }

    /// `date` moved by `steps` periods, back for negative steps. A day that
    /// the month moved to lacks becomes its last day. `None` beyond the
    /// dates chrono knows.
    pub fn step(self, date: NaiveDate, steps: i32) -> Option<NaiveDate> {
        let months = |per_step: u32| Months::new(per_step * steps.unsigned_abs());
        match (self, steps < 0) {
            (Self::Week, _) => date.checked_add_signed(TimeDelta::weeks(steps.into())),
            (Self::Month, false) => date.checked_add_months(months(1)),
            (Self::Month, true) => date.checked_sub_months(months(1)),
            (Self::Year, false) => date.checked_add_months(months(12)),
            (Self::Year, true) => date.checked_sub_months(months(12)),
        }
    }
}

/// The first day of the week `date` lies in, for weeks starting on
/// `first_day`.
pub fn week_start(date: NaiveDate, first_day: Weekday) -> NaiveDate {
    date - Days::new(date.weekday().days_since(first_day).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn ranges() {
        // A Wednesday.
        let today = date(2026, 9, 23);
        assert_eq!(
            Period::Week.range(today, Weekday::Mon),
            (date(2026, 9, 21), date(2026, 9, 27))
        );
        assert_eq!(
            Period::Week.range(today, Weekday::Sun),
            (date(2026, 9, 20), date(2026, 9, 26))
        );
        assert_eq!(
            Period::Week.range(date(2026, 9, 21), Weekday::Mon).0,
            date(2026, 9, 21)
        );
        assert_eq!(
            Period::Month.range(today, Weekday::Mon),
            (date(2026, 9, 1), date(2026, 9, 30))
        );
        assert_eq!(
            Period::Month.range(date(2028, 2, 10), Weekday::Mon),
            (date(2028, 2, 1), date(2028, 2, 29))
        );
        assert_eq!(
            Period::Year.range(today, Weekday::Mon),
            (date(2026, 1, 1), date(2026, 12, 31))
        );
    }

    #[test]
    fn steps() {
        let today = date(2026, 1, 31);
        assert_eq!(Period::Week.step(today, -2), Some(date(2026, 1, 17)));
        assert_eq!(Period::Month.step(today, 1), Some(date(2026, 2, 28)));
        assert_eq!(Period::Month.step(today, -3), Some(date(2025, 10, 31)));
        assert_eq!(Period::Year.step(today, 2), Some(date(2028, 1, 31)));
        assert_eq!(Period::Year.step(today, -1), Some(date(2025, 1, 31)));
        assert_eq!(Period::Year.step(NaiveDate::MAX, 1), None);
    }
}
