use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::TimeDelta;
use gtk::{gdk, glib, graphene, gsk};

use crate::colors::{sea_green, with_alpha};
use crate::format::format_duration;

const HEIGHT: f32 = 320.0;
/// Room for the hour labels of the days on the left and of the week on the
/// right.
const LEFT: f32 = 44.0;
const RIGHT: f32 = 48.0;
const TOP: f32 = 12.0;
/// Room for the day labels.
const BOTTOM: f32 = 40.0;

/// One column of the chart.
#[derive(Debug, Clone)]
pub struct ChartDay {
    pub label: String,
    /// Hours per project, stacked from the bottom.
    pub segments: Vec<(gdk::RGBA, f32)>,
    /// Hours worked, for the running total. `None` for days still to come.
    pub working_hours: Option<f32>,
    /// The share of the week's target hours that falls on this day.
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

    impl ObjectImpl for WeekChart {}

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
            let plot = graphene::Rect::new(
                LEFT,
                TOP,
                widget.width() as f32 - LEFT - RIGHT,
                widget.height() as f32 - TOP - BOTTOM,
            );
            let column = plot.width() / days.len().max(1) as f32;

            // Hours per day on the left scale.
            let highest = days
                .iter()
                .map(|day| {
                    let worked = day.segments.iter().map(|(_, hours)| hours).sum::<f32>();
                    worked.max(day.target_hours)
                })
                .fold(1.0, f32::max);
            // Headroom keeps the targets off the top line.
            let (day_max, day_step) = scale(highest * 1.1);
            let lines = (day_max / day_step).round();
            let y_day = |hours: f32| plot.y() + plot.height() * (1.0 - hours / day_max);

            // The running total and its target on the right scale, which
            // shares the lines of the left one.
            let totals = running_totals(days.iter().map(|day| day.working_hours));
            let targets = running_totals(days.iter().map(|day| Some(day.target_hours)));
            let week_highest = totals.iter().chain(&targets).copied().fold(1.0, f32::max);
            let week_step = step_for(week_highest, lines);
            let week_max = week_step * lines;
            let y_week = |hours: f32| plot.y() + plot.height() * (1.0 - hours / week_max);

            let label_color = with_alpha(&foreground, 0.55);
            for line in 0..=lines as u32 {
                let y = y_day(day_step * line as f32);
                let rect = graphene::Rect::new(plot.x(), y, plot.width(), 1.0);
                snapshot.append_color(&with_alpha(&foreground, 0.12), &rect);
                let text = hours_label(day_step * line as f32);
                self.append_text(snapshot, &text, LEFT - 6.0, y, 1.0, &label_color);
                let text = hours_label(week_step * line as f32);
                let x = plot.x() + plot.width() + 6.0;
                self.append_text(snapshot, &text, x, y, 0.0, &label_color);
            }

            for (index, day) in days.iter().enumerate() {
                let center = plot.x() + column * (index as f32 + 0.5);
                let width = column * 0.55;
                let mut bottom = 0.0;
                for (color, hours) in &day.segments {
                    let top = y_day(bottom + hours);
                    let rect =
                        graphene::Rect::new(center - width / 2.0, top, width, y_day(bottom) - top);
                    snapshot.append_color(color, &rect);
                    bottom += hours;
                }
                if day.target_hours > 0.0 {
                    let y = y_day(day.target_hours);
                    let path = gsk::PathBuilder::new();
                    path.move_to(center - width / 2.0 - 4.0, y);
                    path.line_to(center + width / 2.0 + 4.0, y);
                    snapshot.append_stroke(
                        &path.to_path(),
                        &gsk::Stroke::new(2.0),
                        &with_alpha(&foreground, 0.7),
                    );
                }
                let label_y = plot.y() + plot.height() + BOTTOM / 2.0;
                self.append_text(snapshot, &day.label, center, label_y, 0.5, &foreground);
            }

            // Where the running total should be at the end of each day.
            if targets.last().is_some_and(|target| *target > 0.0) {
                let path = gsk::PathBuilder::new();
                for (index, target) in targets.iter().enumerate() {
                    let point = (plot.x() + column * (index as f32 + 0.5), y_week(*target));
                    if index == 0 {
                        path.move_to(point.0, point.1);
                    } else {
                        path.line_to(point.0, point.1);
                    }
                }
                let stroke = gsk::Stroke::new(1.5);
                stroke.set_dash(&[6.0, 4.0]);
                snapshot.append_stroke(&path.to_path(), &stroke, &with_alpha(&foreground, 0.6));
            }

            let green = sea_green();
            let path = gsk::PathBuilder::new();
            for (index, total) in totals.iter().enumerate() {
                let point = (plot.x() + column * (index as f32 + 0.5), y_week(*total));
                if index == 0 {
                    path.move_to(point.0, point.1);
                } else {
                    path.line_to(point.0, point.1);
                }
                path.add_circle(&graphene::Point::new(point.0, point.1), 3.5);
                path.move_to(point.0, point.1);
            }
            snapshot.append_stroke(&path.to_path(), &gsk::Stroke::new(2.5), &green);
        }
    }

    impl WeekChart {
        /// Draws `text` centred vertically on `y`; `align` 0 puts its left,
        /// 1 its right edge on `x`.
        fn append_text(
            &self,
            snapshot: &gtk::Snapshot,
            text: &str,
            x: f32,
            y: f32,
            align: f32,
            color: &gdk::RGBA,
        ) {
            let layout = self.obj().create_pango_layout(Some(text));
            let (width, height) = layout.pixel_size();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                x - width as f32 * align,
                y - height as f32 / 2.0,
            ));
            snapshot.append_layout(&layout, color);
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// Hours per day and project as stacked bars against the target of each
    /// day, with the running total of the week against the running target.
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

/// The top of a scale that shows `value`, and the step of its lines.
fn scale(value: f32) -> (f32, f32) {
    let step = step_for(value, 5.0);
    ((value / step).ceil() * step, step)
}

/// The smallest step of lines that shows `value` within `lines` lines.
fn step_for(value: f32, lines: f32) -> f32 {
    [1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 20.0, 40.0, 50.0, 100.0]
        .into_iter()
        .find(|step| value / step <= lines)
        .unwrap_or(200.0)
}

/// The sums of `hours` up to each day, until the first `None`.
fn running_totals(hours: impl Iterator<Item = Option<f32>>) -> Vec<f32> {
    hours
        .map_while(|hours| hours)
        .scan(0.0, |total, hours| {
            *total += hours;
            Some(*total)
        })
        .collect()
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
        assert_eq!(scale(40.0), (40.0, 8.0));
        assert_eq!(scale(41.0), (50.0, 10.0));
        assert_eq!(scale(1500.0), (1600.0, 200.0));
    }

    #[test]
    fn totals_run_until_the_first_missing_day() {
        let hours = [Some(1.0), Some(2.5), None, Some(4.0)];
        assert_eq!(running_totals(hours.into_iter()), [1.0, 3.5]);
        assert!(running_totals([None, Some(1.0)].into_iter()).is_empty());
    }
}
