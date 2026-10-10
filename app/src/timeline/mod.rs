mod input;

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Block, Day, Vault, minute_of_day};
use chrono::Local;
use glib::subclass::Signal;
use gtk::{gdk, glib, graphene, gsk, pango};

use crate::colors::{self, sea_green, with_alpha};
use crate::drawing::{append_dot, append_layout, fill_rounded};
use crate::widgets::redraw_animation;

/// Height of one minute. A 15 minute block is just high enough for one line.
const MINUTE_HEIGHT: f32 = 1.6;
/// Room above the first and below the last hour line for its label.
const PADDING: f32 = 12.0;
/// Width of the hour labels.
const LABEL_WIDTH: f32 = 44.0;
/// Where the line runs.
const LINE_X: f32 = 56.0;
const BLOCK_X: f32 = 72.0;
const KNOT_RADIUS: f32 = 5.0;
const CURRENT_KNOT_RADIUS: f32 = 7.0;
/// Half the height of the arrow that marks the time now.
const NOW_ARROW: f32 = 5.0;
/// Height of the edges that change start or end of a block when dragged.
const EDGE: f32 = 6.0;
/// Words of text a block hides from which it shows one more dot.
const DOT_STEPS: [usize; 5] = [1, 10, 25, 50, 100];
const DOT_RADIUS: f32 = 2.0;
/// Distance between the centers of two dots.
const DOT_STEP: f32 = 6.0;
/// Room for the dots right of the heading.
pub(crate) const DOTS_WIDTH: i32 = 32;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Timeline {
        pub blocks: RefCell<Vec<Entry>>,
        /// First and last minute shown, both on full hours.
        pub range: Cell<(u32, u32)>,
        pub slot_minutes: Cell<u32>,
        pub is_today: Cell<bool>,
        /// Index of the selected block in `blocks`.
        pub selected: Cell<Option<usize>>,
        pub drag: Cell<Option<Drag>>,
        /// The span being dragged, or waiting for its block.
        pub pending: Cell<Option<(u32, u32)>>,
        pub reorder: RefCell<Option<Reorder>>,
        /// Where the blocks started, in minutes, when they began to glide
        /// to where `reorder` puts them; empty while they stand still.
        pub glide_from: RefCell<Vec<f32>>,
        /// Where the marked span started when it began to glide to `pending`.
        pub pending_from: Cell<Option<(f32, f32)>>,
        pub glide: OnceCell<adw::TimedAnimation>,
        pub popover: RefCell<Option<gtk::Popover>>,
    }

    #[derive(Debug, Clone, Copy)]
    pub enum Drag {
        /// Selecting free time from `anchor` on.
        New { anchor: u32 },
        /// Changing `part` of the block `index`, grabbed at `minute`.
        Block {
            index: usize,
            part: Part,
            minute: u32,
        },
    }

    /// A dragged block taking the place of another.
    #[derive(Debug, PartialEq, Eq)]
    pub struct Reorder {
        /// The index of the other block.
        pub place: usize,
        /// The spans of all blocks then, as they are shown.
        pub spans: Vec<(u32, u32)>,
    }

    /// Where a block is grabbed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Part {
        Start,
        Body,
        End,
    }

    #[derive(Debug)]
    pub struct Entry {
        pub span: (u32, u32),
        /// `None` for a project the vault does not know.
        pub color: Option<gdk::RGBA>,
        pub child: gtk::Widget,
        /// The lines of text in `child`, shown as far as they fit.
        pub lines: Vec<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "BitLogTimeline";
        type Type = super::Timeline;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Timeline {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted with the index of the block in the day.
                    Signal::builder("block-activated")
                        .param_types([u32::static_type()])
                        .build(),
                    // Emitted with the first and last minute of a span of
                    // free time selected for a new block.
                    Signal::builder("span-selected")
                        .param_types([u32::static_type(), u32::static_type()])
                        .build(),
                    // Emitted with the index of a block dragged to a new
                    // first and last minute.
                    Signal::builder("block-moved")
                        .param_types([u32::static_type(), u32::static_type(), u32::static_type()])
                        .build(),
                    // Emitted with the indices of a block dragged to the
                    // place of another and of that block.
                    Signal::builder("block-reordered")
                        .param_types([u32::static_type(), u32::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_input();

            let glide = redraw_animation(&*self.obj(), true);
            self.glide.set(glide).expect("constructed runs once");

            // Moves the highlight on while the day is open.
            glib::timeout_add_seconds_local(
                30,
                glib::clone!(
                    #[weak(rename_to = timeline)]
                    self.obj(),
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        if timeline.imp().is_today.get() {
                            timeline.queue_draw();
                        }
                        glib::ControlFlow::Continue
                    }
                ),
            );
        }

        fn dispose(&self) {
            for entry in self.blocks.take() {
                entry.child.unparent();
            }
            if let Some(popover) = self.popover.take() {
                popover.unparent();
            }
        }
    }

    impl WidgetImpl for Timeline {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Vertical => {
                    let (first, last) = self.range.get();
                    y_of(first, last as f32) + PADDING
                }
                _ => BLOCK_X + 120.0,
            };
            (size as i32, size as i32, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let (first, _) = self.range.get();
            for (index, entry) in self.blocks.borrow().iter().enumerate() {
                let (start, end) = self.shown_span(index);
                let (min_width, _, _, _) = entry.child.measure(gtk::Orientation::Horizontal, -1);
                let child_width = (width - BLOCK_X as i32).max(min_width);
                let (min_height, _, _, _) =
                    entry.child.measure(gtk::Orientation::Vertical, child_width);
                // Short blocks get their minimum height and are clipped in `snapshot`.
                let height = (((end - start) * MINUTE_HEIGHT) as i32).max(min_height);
                let top = y_of(first, start) as i32;
                entry.child.size_allocate(
                    &gtk::Allocation::new(BLOCK_X as i32, top, child_width, height),
                    -1,
                );
                let area = self.area(index);
                let bottom = area.y() + area.height();
                for line in &entry.lines {
                    let fits = line
                        .compute_bounds(&*self.obj())
                        .is_some_and(|bounds| bounds.y() + bounds.height() <= bottom);
                    line.set_child_visible(fits);
                }
            }
            if let Some(popover) = self.popover.borrow().as_ref() {
                popover.present();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = widget.width() as f32;
            let foreground = widget.color();
            let (first, last) = self.range.get();

            self.snapshot_grid(snapshot, width, &foreground);
            let line =
                graphene::Rect::new(LINE_X - 1.0, 0.0, 2.0, y_of(first, last as f32) + PADDING);
            snapshot.append_color(&with_alpha(&foreground, 0.3), &line);

            let now = self
                .is_today
                .get()
                .then(|| minute_of_day(Local::now().time()));
            let focus_visible = widget
                .root()
                .and_downcast::<gtk::Window>()
                .is_some_and(|window| window.gets_focus_visible());
            for (index, entry) in self.blocks.borrow().iter().enumerate() {
                let (start, end) = entry.span;
                let is_shifted = self.is_shifted(index);
                if is_shifted {
                    snapshot.push_opacity(0.5);
                }
                let is_current = now.is_some_and(|now| (start..end).contains(&now));
                let is_selected = self.selected.get() == Some(index);
                let color = entry.color.unwrap_or_else(|| with_alpha(&foreground, 0.5));
                let area = self.area(index);
                let rounded = gsk::RoundedRect::from_rect(area, 6.0);

                snapshot.push_rounded_clip(&rounded);
                let fill = if is_current || is_selected { 0.4 } else { 0.18 };
                snapshot.append_color(&with_alpha(&color, fill), &area);
                let stripe = graphene::Rect::new(area.x(), area.y(), 4.0, area.height());
                snapshot.append_color(&color, &stripe);
                let hidden_words: usize = entry
                    .lines
                    .iter()
                    .filter(|line| !line.is_child_visible())
                    .map(|line| words(&line.label()))
                    .sum();
                let last_line = entry
                    .lines
                    .iter()
                    .rev()
                    .find(|line| line.is_child_visible())
                    .filter(|_| hidden_words > 0)
                    .and_then(|line| line.compute_bounds(&*widget));
                if let Some(last_line) = last_line {
                    self.snapshot_faded_child(snapshot, entry, &area, &last_line);
                } else {
                    widget.snapshot_child(&entry.child, snapshot);
                }
                snapshot.pop();
                self.snapshot_dots(snapshot, entry, &area, dots(hidden_words), &color);

                let outline = if entry.child.has_focus() && focus_visible {
                    Some(with_alpha(&foreground, 0.6))
                } else if is_selected {
                    Some(color)
                } else {
                    None
                };
                if let Some(outline) = outline {
                    snapshot.append_border(&rounded, &[2.0; 4], &[outline; 4]);
                }

                let (radius, knot_color) = if is_current {
                    (CURRENT_KNOT_RADIUS, sea_green())
                } else {
                    (KNOT_RADIUS, foreground)
                };
                let knot_y = y_of(first, self.shown_span(index).0);
                append_dot(snapshot, LINE_X, knot_y, radius, &knot_color);
                if is_shifted {
                    snapshot.pop();
                }
            }

            if let Some((start, end)) = self.shown_pending() {
                let accent = adw::StyleManager::default().accent_color_rgba();
                let area = self.minutes_area(start, end);
                fill_rounded(snapshot, area, 6.0, &with_alpha(&accent, 0.3));
                let rounded = gsk::RoundedRect::from_rect(area, 6.0);
                snapshot.append_border(&rounded, &[2.0; 4], &[accent; 4]);
            }

            if let Some(now) = now.filter(|now| (first..=last).contains(now)) {
                append_now(
                    snapshot,
                    y_of(first, now as f32),
                    width,
                    &with_alpha(&foreground, 0.5),
                );
            }
        }
    }

    impl Timeline {
        /// Where the block `index` goes, moved aside while a dragged block
        /// takes the place of another.
        pub fn target_span(&self, index: usize) -> (u32, u32) {
            match self.reorder.borrow().as_ref() {
                Some(reorder) => reorder.spans[index],
                None => self.blocks.borrow()[index].span,
            }
        }

        /// Where the block `index` is shown, in minutes, on its way to its
        /// target.
        pub fn shown_span(&self, index: usize) -> (f32, f32) {
            let (start, end) = self.target_span(index);
            let length = (end - start) as f32;
            let start = match self.glide_from.borrow().get(index) {
                Some(&from) => self.glided(from, start),
                None => start as f32,
            };
            (start, start + length)
        }

        /// Where the marked span is shown, in minutes: with the dragged block
        /// while it takes the place of another.
        pub fn shown_pending(&self) -> Option<(f32, f32)> {
            if let Some(Drag::Block { index, .. }) = self.drag.get()
                && self.reorder.borrow().is_some()
            {
                return Some(self.shown_span(index));
            }
            let (start, end) = self.pending.get()?;
            Some(match self.pending_from.get() {
                Some((from_start, from_end)) => {
                    (self.glided(from_start, start), self.glided(from_end, end))
                }
                None => (start as f32, end as f32),
            })
        }

        /// The minute shown on the way from `from` to `to`.
        fn glided(&self, from: f32, to: u32) -> f32 {
            let progress = self.glide.get().expect("set up when constructed").value();
            from + (to as f32 - from) * progress as f32
        }

        /// Lets the blocks and the marked span glide from where they are
        /// shown to their targets.
        pub fn start_glide(&self) {
            let shown = (0..self.blocks.borrow().len())
                .map(|index| self.shown_span(index).0)
                .collect();
            self.glide_from.replace(shown);
            self.pending_from.set(self.shown_pending());
            let glide = self.glide.get().expect("set up when constructed");
            glide.reset();
            glide.play();
        }

        /// Puts the blocks and the marked span at their targets at once.
        pub fn stop_glide(&self) {
            self.glide.get().expect("set up when constructed").skip();
            self.glide_from.take();
            self.pending_from.set(None);
        }

        /// Whether the block `index` is only shown moved aside for another,
        /// dragged block.
        fn is_shifted(&self, index: usize) -> bool {
            let is_dragged = matches!(
                self.drag.get(),
                Some(Drag::Block { index: dragged, .. }) if dragged == index
            );
            !is_dragged && self.target_span(index) != self.blocks.borrow()[index].span
        }

        /// Where the block `index` is drawn.
        pub fn area(&self, index: usize) -> graphene::Rect {
            let (start, end) = self.shown_span(index);
            self.minutes_area(start, end)
        }

        /// Where a block from `start` to `end` is drawn.
        pub fn span_area(&self, (start, end): (u32, u32)) -> graphene::Rect {
            self.minutes_area(start as f32, end as f32)
        }

        fn minutes_area(&self, start: f32, end: f32) -> graphene::Rect {
            let (first, _) = self.range.get();
            graphene::Rect::new(
                BLOCK_X,
                y_of(first, start) + 1.0,
                self.obj().width() as f32 - BLOCK_X,
                (end - start) * MINUTE_HEIGHT - 2.0,
            )
        }

        /// The content of `entry` with `last_line` fading out, as a hint that
        /// more text follows.
        fn snapshot_faded_child(
            &self,
            snapshot: &gtk::Snapshot,
            entry: &Entry,
            area: &graphene::Rect,
            last_line: &graphene::Rect,
        ) {
            let opaque = gdk::RGBA::BLACK;
            let top = last_line.y();
            let bottom = top + last_line.height();
            snapshot.push_mask(gsk::MaskMode::Alpha);
            snapshot.append_color(
                &opaque,
                &graphene::Rect::new(area.x(), area.y(), area.width(), top - area.y()),
            );
            snapshot.append_linear_gradient(
                &graphene::Rect::new(area.x(), top, area.width(), last_line.height()),
                &graphene::Point::new(0.0, top),
                &graphene::Point::new(0.0, bottom),
                &[
                    gsk::ColorStop::new(0.0, opaque),
                    gsk::ColorStop::new(1.0, with_alpha(&opaque, 0.1)),
                ],
            );
            snapshot.pop();
            self.obj().snapshot_child(&entry.child, snapshot);
            snapshot.pop();
        }

        /// `count` dots right of the heading of `entry`, for the text it hides.
        fn snapshot_dots(
            &self,
            snapshot: &gtk::Snapshot,
            entry: &Entry,
            area: &graphene::Rect,
            count: usize,
            color: &gdk::RGBA,
        ) {
            let Some(heading) = entry
                .child
                .first_child()
                .and_then(|heading| heading.compute_bounds(&*self.obj()))
            else {
                return;
            };
            let y = heading.y() + heading.height() / 2.0;
            append_dots(snapshot, area.x() + area.width() - 8.0, y, count, color);
        }

        /// The lines of the slots and the labelled lines of full hours.
        fn snapshot_grid(&self, snapshot: &gtk::Snapshot, width: f32, foreground: &gdk::RGBA) {
            let (first, last) = self.range.get();
            let slot = self.slot_minutes.get();
            for minute in (first..=last).step_by(slot as usize) {
                let y = y_of(first, minute as f32);
                let is_hour = minute % 60 == 0;
                let alpha = if is_hour { 0.2 } else { 0.07 };
                let line = graphene::Rect::new(LINE_X, y, width - LINE_X, 1.0);
                snapshot.append_color(&with_alpha(foreground, alpha), &line);
                if is_hour {
                    let text = format!("{:02}:00", minute / 60 % 24);
                    let layout = self.obj().create_pango_layout(Some(&text));
                    let color = with_alpha(foreground, 0.55);
                    append_layout(snapshot, &layout, (LABEL_WIDTH, y), (1.0, 0.5), &color);
                }
            }
        }
    }
}

glib::wrapper! {
    /// The blocks of a day on a time grid, with the knotted line on the left.
    pub struct Timeline(ObjectSubclass<imp::Timeline>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Timeline {
    pub fn set_day(&self, vault: &Vault, day: &Day, is_today: bool) {
        let imp = self.imp();
        let grid = &vault.config().grid;
        let spans = day.blocks.iter().map(Block::span);
        let first = spans
            .clone()
            .map(|(start, _)| start)
            .chain([minute_of_day(grid.day_start)])
            .min()
            .expect("the grid start is always there");
        let last = spans
            .map(|(_, end)| end)
            .chain([minute_of_day(grid.day_end)])
            .max()
            .expect("the grid end is always there");
        imp.range.set((first / 60 * 60, last.div_ceil(60) * 60));
        imp.slot_minutes.set(grid.slot_minutes);
        imp.is_today.set(is_today);
        imp.selected.set(None);
        imp.drag.set(None);
        imp.pending.set(None);
        imp.reorder.replace(None);
        imp.stop_glide();

        for entry in imp.blocks.take() {
            entry.child.unparent();
        }
        let entries = day
            .blocks
            .iter()
            .map(|block| {
                let project = vault.project(&block.project);
                let (child, lines) = block_content(block, vault.project_name(&block.project));
                child.set_parent(self);
                child.connect_has_focus_notify(glib::clone!(
                    #[weak(rename_to = timeline)]
                    self,
                    move |_| timeline.queue_draw()
                ));
                imp::Entry {
                    span: block.span(),
                    color: project.map(|project| colors::parse(&project.color)),
                    child,
                    lines,
                }
            })
            .collect();
        imp.blocks.replace(entries);
        self.queue_resize();
    }

    /// Marks the block with the index `index` in the day as selected, or none.
    pub fn select(&self, index: Option<usize>) {
        self.imp().selected.set(index);
        self.queue_draw();
    }

    fn activate(&self, index: usize) {
        self.select(Some(index));
        let index = u32::try_from(index).expect("a day has few blocks");
        self.emit_by_name::<()>("block-activated", &[&index]);
    }

    /// Marks one slot of free time from `minute` on for a new block, if it
    /// is free, and returns it.
    pub fn select_free_span(&self, minute: u32) -> Option<(u32, u32)> {
        let imp = self.imp();
        if !imp.is_free(minute) || imp.popover.borrow().is_some() {
            return None;
        }
        let span = imp.free_span(minute, minute);
        imp.set_pending(Some(span));
        Some(span)
    }

    /// Where the span marked for a new block is drawn.
    pub fn pending_area(&self) -> Option<graphene::Rect> {
        let imp = self.imp();
        imp.pending.get().map(|span| imp.span_area(span))
    }

    /// Shows `popover` at the span selected last, which stays marked until
    /// the popover closes.
    pub fn show_popover(&self, popover: &gtk::Popover) {
        let imp = self.imp();
        let span = imp
            .pending
            .get()
            .expect("a popover belongs to a selected span");
        let area = imp.span_area(span);
        popover.set_parent(self);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            area.x() as i32,
            area.y() as i32,
            area.width() as i32,
            area.height() as i32,
        )));
        let previous_focus = self
            .root()
            .and_then(|root| root.focus())
            .map(|widget| widget.downgrade())
            .unwrap_or_default();
        popover.connect_closed(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |popover| {
                let imp = timeline.imp();
                imp.pending.set(None);
                imp.popover.take();
                // Otherwise the focus leaves the popover for whatever GTK
                // finds next, like the title of the block shown.
                if let Some(root) = timeline.root() {
                    match previous_focus.upgrade().filter(|widget| widget.is_mapped()) {
                        Some(widget) => {
                            widget.grab_focus();
                        }
                        None => root.set_focus(None::<&gtk::Widget>),
                    }
                }
                popover.unparent();
                timeline.queue_draw();
            }
        ));
        imp.popover.replace(Some(popover.clone()));
        popover.popup();
    }

    pub fn connect_span_selected(&self, callback: impl Fn(&Self, u32, u32) + 'static) {
        self.connect_closure(
            "span-selected",
            false,
            glib::closure_local!(move |timeline: &Self, start: u32, end: u32| {
                callback(timeline, start, end);
            }),
        );
    }

    pub fn connect_block_moved(&self, callback: impl Fn(&Self, usize, u32, u32) + 'static) {
        self.connect_closure(
            "block-moved",
            false,
            glib::closure_local!(move |timeline: &Self, index: u32, start: u32, end: u32| {
                callback(timeline, index as usize, start, end);
            }),
        );
    }

    pub fn connect_block_reordered(&self, callback: impl Fn(&Self, usize, usize) + 'static) {
        self.connect_closure(
            "block-reordered",
            false,
            glib::closure_local!(move |timeline: &Self, index: u32, place: u32| {
                callback(timeline, index as usize, place as usize);
            }),
        );
    }

    pub fn connect_block_activated(&self, callback: impl Fn(&Self, usize) + 'static) {
        self.connect_closure(
            "block-activated",
            false,
            glib::closure_local!(move |timeline: &Self, index: u32| {
                callback(timeline, index as usize);
            }),
        );
    }
}

/// Title, project and text of `block`, and the labels of its lines of text.
/// Without a title, the project's name stands in for it as well, in italics.
fn block_content(block: &Block, project_name: &str) -> (gtk::Widget, Vec<gtk::Label>) {
    let label = |text: &str, classes: &[&str]| {
        gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(classes)
            .build()
    };
    let heading = gtk::Box::builder()
        .spacing(6)
        .margin_end(DOTS_WIDTH)
        .build();
    if block.title.is_empty() {
        heading.append(&label(project_name, &["heading", "stand-in-title"]));
    } else {
        heading.append(&label(&block.title, &["heading"]));
    }
    heading.append(&label(project_name, &["dim-label"]));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .focusable(true)
        .can_target(false)
        .accessible_role(gtk::AccessibleRole::Button)
        .margin_start(12)
        .margin_end(8)
        .margin_top(2)
        .build();
    content.update_property(&[gtk::accessible::Property::Label(
        if block.title.is_empty() {
            project_name
        } else {
            &block.title
        },
    )]);
    content.append(&heading);
    let lines: Vec<_> = block
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| label(line, &["caption", "dim-label"]))
        .collect();
    for line in &lines {
        content.append(line);
    }
    (content.upcast(), lines)
}

/// Where `minute` lies below the top, for a grid that starts at `first`.
fn y_of(first: u32, minute: f32) -> f32 {
    PADDING + (minute - first as f32) * MINUTE_HEIGHT
}

/// The line across the grid at the time now, `y`, with an arrow at its
/// right end pointing back at it, clear of the knots.
fn append_now(snapshot: &gtk::Snapshot, y: f32, width: f32, color: &gdk::RGBA) {
    let tip = width - 2.0 * NOW_ARROW;
    let line = graphene::Rect::new(LINE_X, y - 1.0, tip - LINE_X, 2.0);
    snapshot.append_color(color, &line);
    // The tip is cut to the height of the line, so the two meet flush.
    let arrow = gsk::PathBuilder::new();
    arrow.move_to(tip, y - 1.0);
    arrow.line_to(width, y - NOW_ARROW);
    arrow.line_to(width, y + NOW_ARROW);
    arrow.line_to(tip, y + 1.0);
    arrow.close();
    snapshot.append_fill(&arrow.to_path(), gsk::FillRule::Winding, color);
}

/// `count` dots in a row ending at `right`, in a lighter `color`, for the
/// text a block hides.
pub(crate) fn append_dots(
    snapshot: &gtk::Snapshot,
    right: f32,
    y: f32,
    count: usize,
    color: &gdk::RGBA,
) {
    for index in 0..count {
        let x = right - DOT_RADIUS - index as f32 * DOT_STEP;
        append_dot(snapshot, x, y, DOT_RADIUS, &with_alpha(color, 0.65));
    }
}

/// How many dots stand for `words` words of hidden text.
fn dots(words: usize) -> usize {
    DOT_STEPS.iter().filter(|&&step| words >= step).count()
}

/// The words in `line`, without Markdown marks such as `-` or `>`.
fn words(line: &str) -> usize {
    line.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_grow_with_hidden_words() {
        assert_eq!(dots(0), 0);
        assert_eq!(dots(1), 1);
        assert_eq!(dots(9), 1);
        assert_eq!(dots(10), 2);
        assert_eq!(dots(49), 3);
        assert_eq!(dots(50), 4);
        assert_eq!(dots(100), 5);
        assert_eq!(dots(1000), 5);
    }

    #[test]
    fn words_skip_markdown_marks() {
        assert_eq!(words("- [ ] Load test on staging"), 4);
        assert_eq!(words("> Keep the limits per key"), 5);
        assert_eq!(words("```"), 0);
        assert_eq!(words("Details in [[Rate limiting]]."), 4);
    }
}
