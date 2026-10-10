//! A week as the calendar charts it: the hours of each day stacked by
//! project.

use chrono::{Days, NaiveDate};
use gtk::graphene;
use gtk::prelude::*;

use super::{append_layout, layout};
use crate::colors::{parse, with_alpha};
use crate::format::format_date;
use crate::week_chart::append_column;

const PLOT_HEIGHT: f32 = 128.0;
/// Room for the names of the days.
const BOTTOM: f32 = 28.0;
pub(super) const HEIGHT: f32 = PLOT_HEIGHT + BOTTOM;
/// The top of the scale, in hours.
const MAX_HOURS: f32 = 10.0;

/// Hours per project for each day from Monday, the weekend free.
const WEEK: [&[f32]; 7] = [
    &[5.0, 1.5, 1.0],
    &[6.0, 1.0, 1.5],
    &[3.5, 2.0, 2.0, 0.5],
    &[6.5, 1.0],
    &[4.0, 1.5, 0.5],
    &[],
    &[],
];
const COLORS: [&str; 4] = ["#3584e4", "#e5a50a", "#33d17a", "#c061cb"];

pub(super) fn snapshot(widget: &gtk::Widget, snapshot: &gtk::Snapshot) {
    let width = widget.width() as f32;
    let foreground = widget.color();
    let y_of = |hours: f32| PLOT_HEIGHT * (1.0 - hours / MAX_HOURS);
    for hours in [0.0, 4.0, 8.0] {
        let line = graphene::Rect::new(0.0, y_of(hours), width, 1.0);
        snapshot.append_color(&with_alpha(&foreground, 0.08), &line);
    }

    let column = width / WEEK.len() as f32;
    // Any Monday: only the names of the days are shown.
    let monday = NaiveDate::from_ymd_opt(2026, 10, 5).expect("the date exists");
    for (index, hours) in WEEK.iter().enumerate() {
        let center = column * (index as f32 + 0.5);
        let bar_width = column * 0.55;
        let segments: Vec<_> = hours
            .iter()
            .zip(COLORS)
            .map(|(hours, hex)| (parse(hex), *hours))
            .collect();
        append_column(
            snapshot,
            &segments,
            center - bar_width / 2.0,
            bar_width,
            &y_of,
        );
        let date = monday + Days::new(index as u64);
        let name = layout(widget, &format_date(date, "%a"), None);
        append_layout(
            snapshot,
            &name,
            (center, PLOT_HEIGHT + BOTTOM / 2.0),
            (0.5, 0.5),
            &foreground,
        );
    }
}
