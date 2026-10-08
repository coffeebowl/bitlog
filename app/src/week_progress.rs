use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::TimeDelta;
use gtk::{glib, graphene, gsk};

use crate::colors::{parse, with_alpha};

/// The bar, and the mark across it.
const BAR_HEIGHT: f32 = 8.0;
const HEIGHT: f32 = 16.0;
/// Fixed, not the accent color, which could be red or green itself. Green
/// and red are as light and as colorful as the blue in Oklab, so that
/// neither weighs more than the other.
const ON_PLAN: &str = "#3584e4";
const AHEAD: &str = "#0fa05c";
const BEHIND: &str = "#d15c53";

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct WeekProgress {
        pub worked: Cell<TimeDelta>,
        pub target: Cell<TimeDelta>,
        pub expected: Cell<Option<TimeDelta>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WeekProgress {
        const NAME: &'static str = "BitLogWeekProgress";
        type Type = super::WeekProgress;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Meter);
        }
    }

    impl ObjectImpl for WeekProgress {}

    impl WidgetImpl for WeekProgress {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Vertical => HEIGHT as i32,
                _ => 0,
            };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let worked = hours(self.worked.get());
            let target = hours(self.target.get());
            // The bar grows with hours beyond the target.
            let end = worked.max(target);
            if end <= 0.0 {
                return;
            }
            let width = widget.width() as f32;
            let x_of = |hours: f32| width * hours / end;
            let top = (HEIGHT - BAR_HEIGHT) / 2.0;
            let span = |from: f32, to: f32| {
                graphene::Rect::new(x_of(from), top, x_of(to) - x_of(from), BAR_HEIGHT)
            };

            let bar = graphene::Rect::new(0.0, top, width, BAR_HEIGHT);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bar, BAR_HEIGHT / 2.0));
            snapshot.append_color(&with_alpha(&widget.color(), 0.15), &bar);
            // Where the week should be by now, the whole target once it is
            // over; a week still to come has no plan to be behind.
            let expected = self.expected.get();
            let plan = expected.map_or(worked, hours);
            snapshot.append_color(&parse(ON_PLAN), &span(0.0, worked.min(plan)));
            // Only hints of green and red; hours missing are not wrong.
            if worked > plan {
                let ahead = with_alpha(&parse(AHEAD), 0.5);
                snapshot.append_color(&ahead, &span(plan, worked));
            } else {
                let behind = with_alpha(&parse(BEHIND), 0.5);
                snapshot.append_color(&behind, &span(worked, plan));
            }
            snapshot.pop();

            // At the end of the bar, the mark would only say the week is over.
            if expected.is_some() && 0.0 < plan && plan < target {
                let mark = graphene::Rect::new(x_of(plan) - 1.0, 0.0, 2.0, HEIGHT);
                let rounded = gsk::RoundedRect::from_rect(mark, 1.0);
                snapshot.push_rounded_clip(&rounded);
                snapshot.append_color(&widget.color(), &mark);
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    /// The hours worked in a week against its target: on plan in blue,
    /// ahead of it in green, behind it in a hint of red, with a mark
    /// where the week should be by today.
    pub struct WeekProgress(ObjectSubclass<imp::WeekProgress>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WeekProgress {
    /// `expected` is where the week should be by today, `None` for a week
    /// still to come.
    pub fn set(&self, worked: TimeDelta, target: TimeDelta, expected: Option<TimeDelta>) {
        let imp = self.imp();
        imp.worked.set(worked);
        imp.target.set(target);
        imp.expected.set(expected);
        self.update_property(&[
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(f64::from(hours(worked.max(target)))),
            gtk::accessible::Property::ValueNow(f64::from(hours(worked))),
        ]);
        self.queue_draw();
    }
}

fn hours(time: TimeDelta) -> f32 {
    time.num_minutes() as f32 / 60.0
}
