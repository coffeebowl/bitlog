use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, NaiveDate, TimeDelta};
use gtk::{gdk, glib, graphene, gsk, pango};

use crate::colors::{pastel, with_alpha};
use crate::format::{format_date, format_duration};

const HEIGHT: f32 = 332.0;
/// Room for the hour labels on the left, at least.
const LEFT: f32 = 44.0;
const RIGHT: f32 = 16.0;
const TOP: f32 = 12.0;
/// Room for the weekdays and, below them, the days of the month.
const BOTTOM: f32 = 56.0;
/// Between the parts of a column.
const GAP: f32 = 2.0;
const RADIUS: f32 = 6.0;

/// One column of the chart.
#[derive(Debug, Clone)]
pub struct ChartDay {
    pub date: NaiveDate,
    pub is_today: bool,
    /// A workday by the preferences; the others are shaded.
    pub is_workday: bool,
    /// The kind of a day that is not a workday, as in "Vacation".
    pub kind: Option<String>,
    /// Hours per project, stacked from the bottom.
    pub segments: Vec<(gdk::RGBA, f32)>,
    /// The share of the week's target hours that falls on this day, which
    /// the scale makes room for.
    pub target_hours: f32,
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct WeekChart {
        pub days: RefCell<Vec<ChartDay>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WeekChart {
        const NAME: &'static str = "BitLogWeekChart";
        type Type = super::WeekChart;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for WeekChart {
        fn constructed(&self) {
            self.parent_constructed();
            adw::StyleManager::default().connect_accent_color_rgba_notify(glib::clone!(
                #[weak(rename_to = chart)]
                self.obj(),
                move |_| chart.queue_draw()
            ));
        }
    }

    impl WidgetImpl for WeekChart {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Vertical => HEIGHT,
                _ => LEFT + RIGHT + 7.0 * 28.0,
            } as i32;
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let foreground = widget.color();
            let days = self.days.borrow();
            // The target of the workdays stands out on the scale, a line of
            // its own if it falls between the others.
            let target = common_target(&days);
            let target_label =
                target.map(|hours| self.layout(&hours_label(hours), Style::SmallBold));
            // Room for the label of the target, which can be longer than the
            // others, as in "7 h 42 min".
            let left = target_label
                .as_ref()
                .map_or(LEFT, |label| LEFT.max(label.pixel_size().0 as f32 + 8.0));
            let plot = graphene::Rect::new(
                left,
                TOP,
                widget.width() as f32 - left - RIGHT,
                widget.height() as f32 - TOP - BOTTOM,
            );
            let column = plot.width() / days.len().max(1) as f32;

            // Hours per day on the left scale.
            let highest = days
                .iter()
                .map(|day| day.worked_hours().max(day.target_hours))
                .fold(1.0, f32::max);
            // Headroom keeps the columns off the top line.
            let (day_max, day_step) = scale(highest * 1.1);
            let y_day = |hours: f32| plot.y() + plot.height() * (1.0 - hours / day_max);

            let target_y = target.map(y_day);

            let below = graphene::Rect::new(
                plot.x(),
                plot.y() + plot.height() + 4.0,
                plot.width(),
                widget.height() as f32 - plot.y() - plot.height() - 4.0,
            );
            append_days_off(snapshot, &days, &below, &foreground);

            let label_color = with_alpha(&foreground, 0.55);
            for line in 0..=(day_max / day_step).round() as u32 {
                let hours = day_step * line as f32;
                let y = y_day(hours);
                if target_y.is_some_and(|target_y| (target_y - y).abs() < 1.0) {
                    continue;
                }
                let rect = graphene::Rect::new(plot.x(), y, plot.width(), 1.0);
                snapshot.append_color(&with_alpha(&foreground, 0.08), &rect);
                // Labels too close to the target's give way to it.
                if target_y.is_none_or(|target_y| (target_y - y).abs() >= 14.0) {
                    let text = self.layout(&hours_label(hours), Style::Small);
                    append_layout(snapshot, &text, left - 6.0, y, 1.0, &label_color);
                }
            }
            if let (Some(label), Some(y)) = (target_label, target_y) {
                let rect = graphene::Rect::new(plot.x(), y, plot.width(), 1.0);
                snapshot.append_color(&with_alpha(&foreground, 0.16), &rect);
                let color = with_alpha(&foreground, 0.8);
                append_layout(snapshot, &label, left - 6.0, y, 1.0, &color);
            }

            for (index, day) in days.iter().enumerate() {
                let center = plot.x() + column * (index as f32 + 0.5);
                let width = column * 0.55;
                append_column(snapshot, &day.segments, center - width / 2.0, width, &y_day);
                self.append_date(snapshot, day, center, plot.y() + plot.height());
                // Above the column, or on the axis for a day not worked.
                if let Some(kind) = &day.kind {
                    let label = self.layout(kind, Style::Small);
                    label.set_width(((column - 4.0) * pango::SCALE as f32) as i32);
                    label.set_ellipsize(pango::EllipsizeMode::End);
                    let y = y_day(day.worked_hours()) - 4.0 - label.pixel_size().1 as f32 / 2.0;
                    append_layout(snapshot, &label, center, y, 0.5, &label_color);
                }
            }
        }
    }

    impl WeekChart {
        fn layout(&self, text: &str, style: Style) -> pango::Layout {
            let layout = self.obj().create_pango_layout(Some(text));
            let attributes = pango::AttrList::new();
            // Small as the caption style class.
            if matches!(style, Style::Small | Style::SmallBold) {
                attributes.insert(pango::AttrFloat::new_scale(0.82));
            }
            if matches!(style, Style::Bold | Style::SmallBold) {
                attributes.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
            }
            layout.set_attributes(Some(&attributes));
            layout
        }

        /// The weekday and the day of the month of `day` below `top`, the
        /// day of today in a circle in the accent color.
        fn append_date(&self, snapshot: &gtk::Snapshot, day: &ChartDay, x: f32, top: f32) {
            let foreground = self.obj().color();
            let weekday = self.layout(&format_date(day.date, "%a"), Style::Small);
            append_layout(
                snapshot,
                &weekday,
                x,
                top + 18.0,
                0.5,
                &with_alpha(&foreground, 0.55),
            );
            let number = self.layout(&day.date.day().to_string(), Style::Bold);
            let center = graphene::Point::new(x, top + 39.0);
            let color = if day.is_today {
                let circle = gsk::PathBuilder::new();
                circle.add_circle(&center, 12.0);
                let accent = adw::StyleManager::default().accent_color_rgba();
                snapshot.append_fill(&circle.to_path(), gsk::FillRule::Winding, &accent);
                gdk::RGBA::WHITE
            } else {
                foreground
            };
            append_layout(snapshot, &number, center.x(), center.y(), 0.5, &color);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Style {
    Small,
    Bold,
    SmallBold,
}

/// Draws `layout` centred vertically on `y`; `align` 0 puts its left, 1 its
/// right edge on `x`.
fn append_layout(
    snapshot: &gtk::Snapshot,
    layout: &pango::Layout,
    x: f32,
    y: f32,
    align: f32,
    color: &gdk::RGBA,
) {
    let (width, height) = layout.pixel_size();
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        x - width as f32 * align,
        y - height as f32 / 2.0,
    ));
    snapshot.append_layout(layout, color);
    snapshot.restore();
}

impl ChartDay {
    fn worked_hours(&self) -> f32 {
        self.segments.iter().map(|(_, hours)| hours).sum()
    }
}

/// The days without work below the axis, in `bounds`, shaded as in the
/// month: days off a little darker than the days that are not workdays.
/// Days in a row make one band with round corners.
fn append_days_off(
    snapshot: &gtk::Snapshot,
    days: &[ChartDay],
    bounds: &graphene::Rect,
    foreground: &gdk::RGBA,
) {
    let shades: Vec<Option<f32>> = days
        .iter()
        .map(|day| match (&day.kind, day.is_workday) {
            (Some(_), _) => Some(0.1),
            (None, false) => Some(0.05),
            (None, true) => None,
        })
        .collect();
    let column = bounds.width() / days.len().max(1) as f32;
    let span = |from: usize, to: usize| {
        let x = bounds.x() + column * from as f32;
        graphene::Rect::new(x, bounds.y(), column * (to - from) as f32, bounds.height())
    };
    let mut start = 0;
    while start < shades.len() {
        if shades[start].is_none() {
            start += 1;
            continue;
        }
        let end = (start..shades.len())
            .find(|index| shades[*index].is_none())
            .unwrap_or(shades.len());
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(span(start, end), RADIUS));
        for (index, shade) in shades.iter().enumerate().take(end).skip(start) {
            let shade = shade.expect("days in a band are shaded");
            snapshot.append_color(&with_alpha(foreground, shade), &span(index, index + 1));
        }
        snapshot.pop();
        start = end;
    }
}

/// Hours per project in pastels of their colors, stacked from the left edge
/// `x` up into a column with round corners and gaps between them. `y` gives
/// the height of hours. The help shows a week the same way.
pub(crate) fn append_column(
    snapshot: &gtk::Snapshot,
    segments: &[(gdk::RGBA, f32)],
    x: f32,
    width: f32,
    y: &impl Fn(f32) -> f32,
) {
    let worked: f32 = segments.iter().map(|(_, hours)| hours).sum();
    let height = y(0.0) - y(worked);
    if height <= 0.0 {
        return;
    }
    let bounds = graphene::Rect::new(x, y(0.0) - height, width, height);
    let radius = RADIUS.min(width / 2.0).min(height / 2.0);
    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, radius));
    let mut hours_below = 0.0;
    for (index, (color, hours)) in segments.iter().enumerate() {
        let top = y(hours_below + hours);
        let bottom = if index == 0 {
            y(hours_below)
        } else {
            y(hours_below) - GAP
        };
        if bottom > top {
            let rect = graphene::Rect::new(x, top, width, bottom - top);
            snapshot.append_color(&pastel(color), &rect);
        }
        hours_below += hours;
    }
    snapshot.pop();
}

glib::wrapper! {
    /// Hours per day and project as stacked bars.
    pub struct WeekChart(ObjectSubclass<imp::WeekChart>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WeekChart {
    pub fn set_week(&self, days: Vec<ChartDay>) {
        self.imp().days.replace(days);
        self.queue_draw();
    }
}

/// The top of a scale that shows `value` in at most four lines above the
/// bottom one, and the step of its lines.
fn scale(value: f32) -> (f32, f32) {
    let step = [1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 20.0, 40.0, 50.0, 100.0]
        .into_iter()
        .find(|step| value / step <= 4.0)
        .unwrap_or(200.0);
    ((value / step).ceil() * step, step)
}

/// The target of the days of the week that have one, if it is the same
/// for all of them.
fn common_target(days: &[ChartDay]) -> Option<f32> {
    let mut targets = days
        .iter()
        .map(|day| day.target_hours)
        .filter(|hours| *hours > 0.0);
    let first = targets.next()?;
    targets
        .all(|hours| (hours - first).abs() < 0.01)
        .then_some(first)
}

fn hours_label(hours: f32) -> String {
    format_duration(TimeDelta::minutes((hours * 60.0).round() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_fits_value_in_few_lines() {
        assert_eq!(scale(0.0), (0.0, 1.0));
        assert_eq!(scale(4.0), (4.0, 1.0));
        assert_eq!(scale(7.5), (8.0, 2.0));
        assert_eq!(scale(9.0), (12.0, 4.0));
        assert_eq!(scale(40.0), (40.0, 10.0));
        assert_eq!(scale(1500.0), (1600.0, 200.0));
    }

    #[test]
    fn target_is_common_only_if_all_workdays_share_it() {
        let week = |targets: [f32; 7]| -> Vec<ChartDay> {
            let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
            (0..7)
                .map(|offset| ChartDay {
                    date: monday + chrono::Days::new(offset as u64),
                    is_today: false,
                    is_workday: true,
                    kind: None,
                    segments: Vec::new(),
                    target_hours: targets[offset],
                })
                .collect()
        };
        let days_off = [7.7, 7.7, 0.0, 7.7, 7.7, 0.0, 0.0];
        assert_eq!(common_target(&week(days_off)), Some(7.7));
        let short_friday = [8.0, 8.0, 8.0, 8.0, 6.0, 0.0, 0.0];
        assert_eq!(common_target(&week(short_friday)), None);
        assert_eq!(common_target(&week([0.0; 7])), None);
    }
}
