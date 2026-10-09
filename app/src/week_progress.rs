use std::cell::{Cell, OnceCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, graphene, gsk};

use crate::colors::{parse, with_alpha};
use crate::week_chart::GLIDE_MS;
use crate::widgets::redraw_animation;

/// The bar, and the mark across it.
const BAR_HEIGHT: f32 = 8.0;
const HEIGHT: f32 = 16.0;
/// Fixed, not the accent color, which could be red or green itself. Green
/// and red are as light and as colorful as the blue in Oklab, so that
/// neither weighs more than the other.
const ON_PLAN: &str = "#3584e4";
const AHEAD: &str = "#0fa05c";
const BEHIND: &str = "#d15c53";

/// What the bar shows, as shares of its length, which glide from one week
/// to another as they are seen; all 0 for a week without a target.
#[derive(Debug, Default, Clone, Copy)]
struct Bar {
    worked: f32,
    target: f32,
    /// Where the week should be by now, the whole target once it is over.
    plan: f32,
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct WeekProgress {
        /// The bar as it was shown when it was last set, which `glide`
        /// takes it from.
        pub(super) from: Cell<Bar>,
        pub(super) to: Cell<Bar>,
        /// Whether the week has come as far as today, with a mark for it.
        pub has_mark: Cell<bool>,
        pub glide: OnceCell<adw::TimedAnimation>,
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

    impl ObjectImpl for WeekProgress {
        fn constructed(&self) {
            self.parent_constructed();
            let glide = redraw_animation(&*self.obj(), GLIDE_MS);
            self.glide.set(glide).expect("constructed runs once");
        }
    }

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
            let Bar {
                worked,
                target,
                plan,
            } = widget.shown();
            let width = widget.width() as f32;
            let top = (HEIGHT - BAR_HEIGHT) / 2.0;
            let bar = graphene::Rect::new(0.0, top, width, BAR_HEIGHT);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bar, BAR_HEIGHT / 2.0));
            snapshot.append_color(&with_alpha(&widget.color(), 0.15), &bar);
            // A week without a target keeps the empty bar, so that the
            // chart below does not move.
            if target <= 0.0 {
                snapshot.pop();
                return;
            }
            let x_of = |share: f32| width * share;
            let span = |from: f32, to: f32| {
                graphene::Rect::new(x_of(from), top, x_of(to) - x_of(from), BAR_HEIGHT)
            };
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
            if self.has_mark.get() && 0.0 < plan && plan < target {
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
    /// Hours; `expected` is where the week should be by today, `None` for a
    /// week still to come.
    pub fn set(&self, worked: f32, target: f32, expected: Option<f32>) {
        let imp = self.imp();
        imp.from.set(self.shown());
        // The bar grows with hours beyond the target.
        let end = worked.max(target);
        let share = |hours: f32| if target > 0.0 { hours / end } else { 0.0 };
        // A week still to come has no plan to be behind.
        let plan = expected.unwrap_or(worked);
        imp.to.set(Bar {
            worked: share(worked),
            target: share(target),
            plan: share(plan),
        });
        imp.has_mark.set(expected.is_some());
        self.update_property(&[
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(f64::from(end)),
            gtk::accessible::Property::ValueNow(f64::from(worked)),
        ]);
        if let Some(glide) = imp.glide.get() {
            glide.play();
        }
    }

    /// The bar on its way from where it was to where it was last set.
    fn shown(&self) -> Bar {
        let imp = self.imp();
        let progress = imp.glide.get().map_or(1.0, |glide| glide.value() as f32);
        let (from, to) = (imp.from.get(), imp.to.get());
        let between = |from: f32, to: f32| from + (to - from) * progress;
        Bar {
            worked: between(from.worked, to.worked),
            target: between(from.target, to.target),
            plan: between(from.plan, to.plan),
        }
    }
}
