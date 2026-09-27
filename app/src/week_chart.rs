use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::TimeDelta;
use gtk::{gdk, glib, graphene, gsk};

use crate::colors::{sea_green, with_alpha};
use crate::format::format_duration;

const HEIGHT: f32 = 320.0;
/// Room for the hour labels on the left and the target label on the right.
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
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct WeekChart {
        pub days: RefCell<Vec<ChartDay>>,
        pub target_hours: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WeekChart {
        const NAME: &'static str = "KnotbookWeekChart";
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
                .map(|day| day.segments.iter().map(|(_, hours)| hours).sum::<f32>())
                .fold(1.0, f32::max);
            let (day_max, step) = scale(highest);
            let y_day = |hours: f32| plot.y() + plot.height() * (1.0 - hours / day_max);
            let mut hours = 0.0;
            while hours <= day_max {
                let y = y_day(hours);
                let line = graphene::Rect::new(plot.x(), y, plot.width(), 1.0);
                snapshot.append_color(&with_alpha(&foreground, 0.12), &line);
                let text = hours_label(hours);
                self.append_text(
                    snapshot,
                    &text,
                    LEFT - 6.0,
                    y,
                    1.0,
                    &with_alpha(&foreground, 0.55),
                );
                hours += step;
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
                let label_y = plot.y() + plot.height() + BOTTOM / 2.0;
                self.append_text(snapshot, &day.label, center, label_y, 0.5, &foreground);
            }

            // The running total and the target on the right scale.
            let target = self.target_hours.get();
            let totals: Vec<f32> = days
                .iter()
                .map_while(|day| day.working_hours)
                .scan(0.0, |total, hours| {
                    *total += hours;
                    Some(*total)
                })
                .collect();
            let reached = totals.last().copied().unwrap_or(0.0);
            // Headroom keeps the target off the top line of the day scale.
            let (week_max, _) = scale(target.max(reached).max(1.0) * 1.1);
            let y_week = |hours: f32| plot.y() + plot.height() * (1.0 - hours / week_max);

            if target > 0.0 {
                let y = y_week(target);
                let path = gsk::PathBuilder::new();
                path.move_to(plot.x(), y);
                path.line_to(plot.x() + plot.width(), y);
                let stroke = gsk::Stroke::new(1.5);
                stroke.set_dash(&[6.0, 4.0]);
                snapshot.append_stroke(&path.to_path(), &stroke, &with_alpha(&foreground, 0.6));
                let text = hours_label(target);
                let x = plot.x() + plot.width() + 6.0;
                self.append_text(snapshot, &text, x, y, 0.0, &foreground);
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
    /// Hours per day and project as stacked bars, with the running total of
    /// the week against its target.
    pub struct WeekChart(ObjectSubclass<imp::WeekChart>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WeekChart {
    pub fn set_week(&self, days: Vec<ChartDay>, target_hours: f32) {
        let imp = self.imp();
        imp.days.replace(days);
        imp.target_hours.set(target_hours);
        self.queue_draw();
    }
}

/// The top of a scale that shows `value`, and the step of its lines.
fn scale(value: f32) -> (f32, f32) {
    let step = [1.0, 2.0, 4.0, 5.0, 8.0, 10.0, 20.0, 40.0, 50.0]
        .into_iter()
        .find(|step| value / step <= 5.0)
        .unwrap_or(100.0);
    ((value / step).ceil() * step, step)
}

fn hours_label(hours: f32) -> String {
    format_duration(TimeDelta::minutes((hours * 60.0).round() as i64))
}
