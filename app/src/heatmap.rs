use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Days, NaiveDate, TimeDelta, Weekday};
use gettextrs::gettext;
use gtk::{gdk, glib, graphene, gsk};
use knotbook_core::week_start;

use crate::colors::with_alpha;
use crate::format::{format_date, format_duration, format_full_date};

/// Weeks shown for the last 12 months, and before anything is shown.
const WEEKS: u32 = 53;
/// The size of a day by default, small enough for a year to fit the width
/// of a page.
const CELL: f32 = 8.0;
/// The smallest a day gets in a single row, in a narrow window.
const MIN_CELL: f32 = 4.0;
/// Room for the labels.
const TOP: f32 = 16.0;
/// The space between a week's line and its number.
const LABEL_PAD: f32 = 3.0;

/// The days shown and how much they hold.
#[derive(Debug, Clone)]
pub struct Activity {
    /// The first day of the first week, which may lie before `first`.
    start: NaiveDate,
    /// The first and last day shown.
    first: NaiveDate,
    last: NaiveDate,
    days: BTreeMap<NaiveDate, TimeDelta>,
    color: gdk::RGBA,
}

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Heatmap)]
    pub struct Heatmap {
        pub activity: RefCell<Option<Activity>>,
        /// The width and height of a day; in a single row the most it grows
        /// to, as it shrinks to the width it gets.
        #[property(get, set = Self::set_cell_size, minimum = 4.0, default = CELL)]
        pub cell_size: Cell<f32>,
        /// Whether the days stand in one row instead of a column per week.
        #[property(get, set = Self::set_single_row)]
        pub single_row: Cell<bool>,
    }

    impl Default for Heatmap {
        fn default() -> Self {
            Self {
                activity: RefCell::default(),
                cell_size: Cell::new(CELL),
                single_row: Cell::default(),
            }
        }
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

    #[glib::derived_properties]
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
        fn set_cell_size(&self, size: f32) {
            self.cell_size.set(size);
            self.obj().queue_resize();
        }

        fn set_single_row(&self, single_row: bool) {
            self.single_row.set(single_row);
            self.obj().queue_resize();
        }
    }

    impl WidgetImpl for Heatmap {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            if self.single_row.get() {
                gtk::SizeRequestMode::HeightForWidth
            } else {
                gtk::SizeRequestMode::ConstantSize
            }
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let widget = self.obj();
            let activity = self.activity.borrow();
            let (minimum, natural) = match (orientation, &*activity) {
                // Days as big as the width allows, or without a width from
                // the smallest to the biggest.
                (gtk::Orientation::Vertical, Some(activity))
                    if widget.single_row() && for_size >= 0 =>
                {
                    let size = TOP + widget.cell_and_gap(activity, Some(for_size as f32)).0;
                    (size, size)
                }
                (gtk::Orientation::Vertical, _) if widget.single_row() => {
                    (TOP + MIN_CELL, TOP + widget.cell_size())
                }
                (gtk::Orientation::Vertical, _) => {
                    let (cell, gap) = (widget.cell_size(), grid_gap(widget.cell_size()));
                    let size = TOP + 7.0 * (cell + gap) - gap;
                    (size, size)
                }
                (_, Some(activity)) if widget.single_row() => {
                    let (units, room) = (row_units(activity), widget.label_room());
                    (units * MIN_CELL + room, units * widget.cell_size() + room)
                }
                (_, Some(activity)) => {
                    let size = widget.origin(activity, activity.last, None).0 + widget.cell_size();
                    (size, size)
                }
                (_, None) if widget.single_row() => (MIN_CELL, widget.cell_size()),
                (_, None) => {
                    let (cell, gap) = (widget.cell_size(), grid_gap(widget.cell_size()));
                    let size = WEEKS as f32 * (cell + gap) - gap;
                    (size, size)
                }
            };
            (minimum.ceil() as i32, natural.ceil() as i32, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(activity) = &*self.activity.borrow() else {
                return;
            };
            let width = Some(widget.width() as f32);
            let (cell_size, gap) = widget.cell_and_gap(activity, width);
            let empty = with_alpha(&widget.color(), 0.08);
            for date in activity.dates() {
                let (x, y) = widget.origin(activity, date, width);
                if !widget.single_row() && widget.starts_month(activity, date) {
                    let layout = widget.small_layout(&format_date(date, &gettext("%b")), 80);
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
                let cell = gsk::RoundedRect::from_rect(
                    graphene::Rect::new(x, y, cell_size, cell_size),
                    gap,
                );
                snapshot.push_rounded_clip(&cell);
                snapshot.append_color(&color, cell.bounds());
                snapshot.pop();
            }
            if widget.single_row() {
                widget.snapshot_weeks(snapshot, activity);
            }
        }
    }
}

glib::wrapper! {
    /// A square per day, one column per week or all in a row, the more time
    /// was spent the stronger its color.
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

    /// The first and last day of the last `days` days up to `today`.
    pub fn last_days(today: NaiveDate, days: u64) -> (NaiveDate, NaiveDate) {
        (today - Days::new(days - 1), today)
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
        self.imp().activity.replace(Some(Activity {
            start,
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

    /// The size of a day and the space between days, which grows with it.
    /// In a single row, days shrink to fit `width`, if given.
    fn cell_and_gap(&self, activity: &Activity, width: Option<f32>) -> (f32, f32) {
        let most = self.cell_size();
        if self.single_row() {
            let cell = width.map_or(most, |width| {
                ((width - self.label_room()) / row_units(activity)).clamp(MIN_CELL, most)
            });
            (cell, cell / 4.0)
        } else {
            (most, grid_gap(most))
        }
    }

    /// Where the square of `date` starts, in `width`: in a column per week,
    /// or in a single row.
    fn origin(&self, activity: &Activity, date: NaiveDate, width: Option<f32>) -> (f32, f32) {
        let (cell, gap) = self.cell_and_gap(activity, width);
        if self.single_row() {
            let column = days_between(activity.first, date);
            (column as f32 * (cell + gap), TOP)
        } else {
            let day = days_between(activity.start, date);
            let (week, weekday) = (day / 7, day % 7);
            (
                week as f32 * (cell + gap),
                TOP + weekday as f32 * (cell + gap),
            )
        }
    }

    /// Whether the month's name stands above `date`, in a column per week:
    /// above the first week that starts in the month.
    fn starts_month(&self, activity: &Activity, date: NaiveDate) -> bool {
        date.weekday() == activity.start.weekday() && date.day() <= 7
    }

    /// Marks the weeks of a single row: a line between two weeks, up to
    /// the number of the calendar week beside it, where it has room.
    fn snapshot_weeks(&self, snapshot: &gtk::Snapshot, activity: &Activity) {
        let width = self.width() as f32;
        let (cell, gap) = self.cell_and_gap(activity, Some(width));
        let line_color = with_alpha(&self.color(), 0.3);
        let label_color = with_alpha(&self.color(), 0.6);
        // The first day shown of each week.
        let firsts: Vec<_> = activity
            .dates()
            .filter(|date| *date == activity.first || date.weekday() == activity.start.weekday())
            .collect();
        // Where the line before a week stands, in the space between days.
        let line = |date: NaiveDate| {
            (date != activity.first)
                .then(|| (self.origin(activity, date, Some(width)).0 - gap / 2.0).round())
        };
        for (index, first) in firsts.iter().enumerate() {
            if let Some(x) = line(*first) {
                snapshot.append_color(&line_color, &graphene::Rect::new(x, 0.0, 1.0, TOP + cell));
            }
            let left = line(*first).map_or(0.0, |x| x + 1.0 + LABEL_PAD);
            let right = firsts
                .get(index + 1)
                .and_then(|next| line(*next))
                .map_or(width, |x| x - LABEL_PAD);
            let layout = self.small_layout(&week_label(activity, *first), 70);
            if layout.pixel_size().0 as f32 <= right - left {
                snapshot.save();
                snapshot.translate(&graphene::Point::new(left, 0.0));
                snapshot.append_layout(&layout, &label_color);
                snapshot.restore();
            }
        }
    }

    /// The room a single row keeps after its last day, so that the number
    /// of a week that has only begun fits: as wide as the widest number.
    fn label_room(&self) -> f32 {
        // Translators: A calendar week above its days, as in "W39".
        let widest = gettext("W{week}").replace("{week}", "53");
        self.small_layout(&widest, 70).pixel_size().0 as f32 + 1.0 + LABEL_PAD
    }

    /// `text` in the widget's font at `percent` of its size.
    fn small_layout(&self, text: &str, percent: i32) -> gtk::pango::Layout {
        let layout = self.create_pango_layout(Some(text));
        let mut font = layout.context().font_description().unwrap_or_default();
        font.set_size(font.size() * percent / 100);
        layout.set_font_description(Some(&font));
        layout
    }

    /// The day under `x`, `y` with the time spent on it.
    fn day_at(&self, x: f32, y: f32) -> Option<(NaiveDate, TimeDelta)> {
        let activity = self.imp().activity.borrow();
        let activity = activity.as_ref()?;
        let width = Some(self.width() as f32);
        let cell = self.cell_and_gap(activity, width).0;
        let date = activity.dates().find(|date| {
            let (left, top) = self.origin(activity, *date, width);
            (left..left + cell).contains(&x) && (top..top + cell).contains(&y)
        })?;
        let time = activity.days.get(&date).copied().unwrap_or_default();
        Some((date, time))
    }
}

impl Activity {
    /// The days shown, from the first to the last.
    fn dates(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        self.first.iter_days().take_while(|date| *date <= self.last)
    }
}

/// The space between days in a column per week.
fn grid_gap(cell: f32) -> f32 {
    (cell / 4.0).round()
}

/// The width of the days of a single row in days: the days and the space
/// between them of a quarter day each.
fn row_units(activity: &Activity) -> f32 {
    let days = days_between(activity.first, activity.last) as f32 + 1.0;
    (days - 1.0).mul_add(0.25, days)
}

/// The number of the calendar week of `date`, the ISO week most of its days
/// are in, also for weeks that start on Sunday.
fn week_label(activity: &Activity, date: NaiveDate) -> String {
    let week = (week_start(date, activity.start.weekday()) + Days::new(3))
        .iso_week()
        .week();
    // Translators: A calendar week above its days, as in "W39".
    gettext("W{week}").replace("{week}", &week.to_string())
}

/// The days from `first` to `last`, which is not before it.
fn days_between(first: NaiveDate, last: NaiveDate) -> u32 {
    u32::try_from((last - first).num_days()).expect("last is not before first")
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
