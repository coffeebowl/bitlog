use std::cell::RefCell;
use std::collections::BTreeMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Days, NaiveDate, TimeDelta, Weekday};
use gettextrs::gettext;
use gtk::{gdk, glib, graphene, gsk};
use knotbook_core::week_start;

use crate::format::{format_date, format_duration, format_full_date};

/// Weeks shown for the last 12 months, and before anything is shown.
const WEEKS: u32 = 53;
/// Small enough for a year to fit the width of a page.
const CELL: f32 = 8.0;
const GAP: f32 = 2.0;
/// Room for the month labels.
const TOP: f32 = 16.0;

/// The days shown and how much they hold.
#[derive(Debug, Clone)]
pub struct Activity {
    /// The first day of the first week, which may lie before `first`.
    start: NaiveDate,
    weeks: u32,
    /// The first and last day shown.
    first: NaiveDate,
    last: NaiveDate,
    days: BTreeMap<NaiveDate, TimeDelta>,
    color: gdk::RGBA,
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Heatmap {
        pub activity: RefCell<Option<Activity>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Heatmap {
        const NAME: &'static str = "KnotbookHeatmap";
        type Type = super::Heatmap;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Heatmap {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_has_tooltip(true);
            widget.update_property(&[gtk::accessible::Property::Label(&gettext(
                "Time spent per day",
            ))]);
            widget.connect_query_tooltip(|widget, x, y, _, tooltip| {
                let Some((date, time)) = widget.day_at(x as f32, y as f32) else {
                    return false;
                };
                let time = if time.is_zero() {
                    gettext("Nothing logged")
                } else {
                    format_duration(time)
                };
                tooltip.set_text(Some(&format!("{} · {time}", format_full_date(date))));
                true
            });
        }
    }

    impl Heatmap {
        fn weeks(&self) -> u32 {
            self.activity
                .borrow()
                .as_ref()
                .map_or(WEEKS, |activity| activity.weeks)
        }
    }

    impl WidgetImpl for Heatmap {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Vertical => TOP + 7.0 * (CELL + GAP) - GAP,
                _ => self.weeks() as f32 * (CELL + GAP) - GAP,
            } as i32;
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(activity) = &*self.activity.borrow() else {
                return;
            };
            let empty = with_alpha(&widget.color(), 0.08);
            let mut month_shown = None;
            for week in 0..activity.weeks {
                let x = week as f32 * (CELL + GAP);
                for weekday in 0..7 {
                    let date = activity.start + Days::new((week * 7 + weekday).into());
                    if date > activity.last {
                        break;
                    }
                    if date < activity.first {
                        continue;
                    }
                    // The month's name above the first week that starts in it.
                    if weekday == 0 && month_shown != Some(date.month()) && date.day() <= 7 {
                        month_shown = Some(date.month());
                        let layout =
                            widget.create_pango_layout(Some(&format_date(date, &gettext("%b"))));
                        layout.set_font_description(Some(&small_font(&layout)));
                        snapshot.save();
                        snapshot.translate(&graphene::Point::new(x, 0.0));
                        snapshot.append_layout(&layout, &with_alpha(&widget.color(), 0.6));
                        snapshot.restore();
                    }
                    let time = activity.days.get(&date).copied().unwrap_or_default();
                    let color = match level(time) {
                        0.0 => empty,
                        alpha => with_alpha(&activity.color, alpha),
                    };
                    let y = TOP + weekday as f32 * (CELL + GAP);
                    let cell =
                        gsk::RoundedRect::from_rect(graphene::Rect::new(x, y, CELL, CELL), 2.0);
                    snapshot.push_rounded_clip(&cell);
                    snapshot.append_color(&color, cell.bounds());
                    snapshot.pop();
                }
            }
        }
    }
}

glib::wrapper! {
    /// A square per day, one column per week, the more time was spent the
    /// stronger its color.
    pub struct Heatmap(ObjectSubclass<imp::Heatmap>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Heatmap {
    /// The first and last day of the last 12 months up to `today`, in whole
    /// weeks starting on `first_day`.
    pub fn last_12_months(today: NaiveDate, first_day: Weekday) -> (NaiveDate, NaiveDate) {
        let first = week_start(today, first_day) - Days::new(u64::from(WEEKS - 1) * 7);
        (first, today)
    }

    /// Shows `days`, the time spent per day, in `color`, from `first` to
    /// `last`, with weeks starting on `first_day`.
    pub fn show(
        &self,
        days: &[(NaiveDate, TimeDelta)],
        color: gdk::RGBA,
        (first, last): (NaiveDate, NaiveDate),
        first_day: Weekday,
    ) {
        let start = week_start(first, first_day);
        let weeks = u32::try_from((last - start).num_days() / 7 + 1).expect("last is after first");
        self.imp().activity.replace(Some(Activity {
            start,
            weeks,
            first,
            last,
            days: days
                .iter()
                .copied()
                .filter(|(date, _)| (first..=last).contains(date))
                .collect(),
            color,
        }));
        self.queue_resize();
        self.queue_draw();
    }

    /// The day under `x`, `y` with the time spent on it.
    fn day_at(&self, x: f32, y: f32) -> Option<(NaiveDate, TimeDelta)> {
        let activity = self.imp().activity.borrow();
        let activity = activity.as_ref()?;
        let (week, weekday) = ((x / (CELL + GAP)) as u32, ((y - TOP) / (CELL + GAP)) as u32);
        if x < 0.0 || y < TOP || week >= activity.weeks || weekday >= 7 {
            return None;
        }
        let date = activity.start + Days::new((week * 7 + weekday).into());
        let time = activity.days.get(&date).copied().unwrap_or_default();
        (activity.first..=activity.last)
            .contains(&date)
            .then_some((date, time))
    }
}

/// How strong the color of a day is: none for no time, then in steps up to
/// full for four hours or more.
fn level(time: TimeDelta) -> f32 {
    match time.num_minutes() {
        0 => 0.0,
        1..60 => 0.3,
        60..120 => 0.5,
        120..240 => 0.75,
        _ => 1.0,
    }
}

fn with_alpha(color: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}

/// The widget's font at 80 %, for the month labels.
fn small_font(layout: &gtk::pango::Layout) -> gtk::pango::FontDescription {
    let mut font = layout.context().font_description().unwrap_or_default();
    font.set_size(font.size() * 4 / 5);
    font
}
