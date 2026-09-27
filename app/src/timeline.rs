use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::Local;
use glib::subclass::Signal;
use gtk::{gdk, glib, graphene, gsk, pango};
use knotbook_core::{Block, Day, Vault, minute_of_day};

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
/// Height of the edges that change start or end of a block when dragged.
const EDGE: f32 = 6.0;
/// The accent colour of the brand, used sparingly.
pub const SEA_GREEN: &str = "#3ba99c";

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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "KnotbookTimeline";
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
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let timeline = self.obj();

            // Blocks do not take pointer input themselves: short ones are
            // allocated taller than they are drawn.
            let click = gtk::GestureClick::new();
            click.connect_released(glib::clone!(
                #[weak]
                timeline,
                move |_, _, x, y| {
                    if let Some((index, _)) = timeline.imp().hit(x as f32, y as f32) {
                        timeline.imp().blocks.borrow()[index].child.grab_focus();
                        timeline.activate(index);
                    }
                }
            ));
            timeline.add_controller(click);

            // Dragging over free time, or just clicking it, selects a span
            // of whole slots for a new block. Dragging a block moves it,
            // dragging its edges changes its start or end. On touch
            // screens dragging scrolls instead.
            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin(glib::clone!(
                #[weak]
                timeline,
                move |drag, x, y| {
                    let imp = timeline.imp();
                    let is_touch = drag
                        .device()
                        .is_some_and(|device| device.source() == gdk::InputSource::Touchscreen);
                    let minute = imp.minute_at(y as f32);
                    let grabbed = if is_touch || imp.popover.borrow().is_some() {
                        None
                    } else if let Some((index, part)) = imp.hit(x as f32, y as f32) {
                        Some(Drag::Block {
                            index,
                            part,
                            minute,
                        })
                    } else {
                        imp.is_free(minute).then_some(Drag::New { anchor: minute })
                    };
                    match grabbed {
                        Some(grabbed) => {
                            imp.drag.set(Some(grabbed));
                            imp.drag_to(minute);
                        }
                        None => {
                            drag.set_state(gtk::EventSequenceState::Denied);
                        }
                    }
                }
            ));
            drag.connect_drag_update(glib::clone!(
                #[weak]
                timeline,
                move |drag, _, dy| {
                    if let Some((_, start_y)) = drag.start_point() {
                        let imp = timeline.imp();
                        imp.drag_to(imp.minute_at((start_y + dy) as f32));
                    }
                }
            ));
            drag.connect_drag_end(glib::clone!(
                #[weak]
                timeline,
                move |_, _, _| {
                    let imp = timeline.imp();
                    let Some(drag) = imp.drag.take() else {
                        return;
                    };
                    let (start, end) = imp.pending.get().expect("a drag always has a span");
                    match drag {
                        // The span stays marked while its project is chosen.
                        Drag::New { .. } => {
                            timeline.emit_by_name::<()>("span-selected", &[&start, &end]);
                        }
                        Drag::Block { index, .. } => {
                            imp.set_pending(None);
                            if imp.blocks.borrow()[index].span != (start, end) {
                                let index = u32::try_from(index).expect("a day has few blocks");
                                timeline.emit_by_name::<()>("block-moved", &[&index, &start, &end]);
                            }
                        }
                    }
                }
            ));
            timeline.add_controller(drag);

            let long_press = gtk::GestureLongPress::builder().touch_only(true).build();
            long_press.connect_pressed(glib::clone!(
                #[weak]
                timeline,
                move |_, _, y| {
                    let minute = timeline.imp().minute_at(y as f32);
                    if let Some((start, end)) = timeline.select_free_span(minute) {
                        timeline.emit_by_name::<()>("span-selected", &[&start, &end]);
                    }
                }
            ));
            timeline.add_controller(long_press);

            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion(glib::clone!(
                #[weak]
                timeline,
                move |_, x, y| {
                    let cursor = match timeline.imp().hit(x as f32, y as f32) {
                        Some((_, Part::Body)) => Some("grab"),
                        Some(_) => Some("ns-resize"),
                        None => None,
                    };
                    timeline.set_cursor_from_name(cursor);
                }
            ));
            timeline.add_controller(motion);

            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                timeline,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| {
                    let activates =
                        matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space);
                    let focused = timeline
                        .imp()
                        .blocks
                        .borrow()
                        .iter()
                        .position(|entry| entry.child.has_focus());
                    match focused {
                        Some(index) if activates => {
                            timeline.activate(index);
                            glib::Propagation::Stop
                        }
                        _ => glib::Propagation::Proceed,
                    }
                }
            ));
            timeline.add_controller(keys);

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
                    y_of(first, last) + PADDING
                }
                _ => BLOCK_X + 120.0,
            };
            (size as i32, size as i32, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let (first, _) = self.range.get();
            for entry in self.blocks.borrow().iter() {
                let (start, end) = entry.span;
                let (min_width, _, _, _) = entry.child.measure(gtk::Orientation::Horizontal, -1);
                let child_width = (width - BLOCK_X as i32).max(min_width);
                let (min_height, _, _, _) =
                    entry.child.measure(gtk::Orientation::Vertical, child_width);
                // Short blocks get their minimum height and are clipped in `snapshot`.
                let height = (block_height(start, end) as i32).max(min_height);
                let top = y_of(first, start) as i32;
                entry.child.size_allocate(
                    &gtk::Allocation::new(BLOCK_X as i32, top, child_width, height),
                    -1,
                );
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
            let line = graphene::Rect::new(LINE_X - 1.0, 0.0, 2.0, y_of(first, last) + PADDING);
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
                let (start, _) = entry.span;
                let is_current = now.is_some_and(|now| (entry.span.0..entry.span.1).contains(&now));
                let is_selected = self.selected.get() == Some(index);
                let color = entry.color.unwrap_or_else(|| with_alpha(&foreground, 0.5));
                let area = self.area(entry);
                let rounded = gsk::RoundedRect::from_rect(area, 6.0);

                snapshot.push_rounded_clip(&rounded);
                let fill = if is_current || is_selected { 0.4 } else { 0.18 };
                snapshot.append_color(&with_alpha(&color, fill), &area);
                let stripe = graphene::Rect::new(area.x(), area.y(), 4.0, area.height());
                snapshot.append_color(&color, &stripe);
                widget.snapshot_child(&entry.child, snapshot);
                snapshot.pop();

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
                    let color = gdk::RGBA::parse(SEA_GREEN).expect("the colour is valid");
                    (CURRENT_KNOT_RADIUS, color)
                } else {
                    (KNOT_RADIUS, foreground)
                };
                append_knot(snapshot, y_of(first, start), radius, &knot_color);
            }

            if let Some(span) = self.pending.get() {
                let accent = adw::StyleManager::default().accent_color_rgba();
                let rounded = gsk::RoundedRect::from_rect(self.span_area(span), 6.0);
                snapshot.push_rounded_clip(&rounded);
                snapshot.append_color(&with_alpha(&accent, 0.3), rounded.bounds());
                snapshot.pop();
                snapshot.append_border(&rounded, &[2.0; 4], &[accent; 4]);
            }
        }
    }

    impl Timeline {
        /// Where `entry` is drawn.
        pub fn area(&self, entry: &Entry) -> graphene::Rect {
            self.span_area(entry.span)
        }

        /// Where a block from `start` to `end` is drawn.
        pub fn span_area(&self, (start, end): (u32, u32)) -> graphene::Rect {
            let (first, _) = self.range.get();
            graphene::Rect::new(
                BLOCK_X,
                y_of(first, start) + 1.0,
                self.obj().width() as f32 - BLOCK_X,
                block_height(start, end) - 2.0,
            )
        }

        /// The minute shown at the height `y`, within the grid.
        pub fn minute_at(&self, y: f32) -> u32 {
            let (first, last) = self.range.get();
            let offset = ((y - PADDING) / MINUTE_HEIGHT).max(0.0) as u32;
            (first + offset).min(last)
        }

        /// The block drawn at `x`, `y`, and where it is grabbed there.
        pub fn hit(&self, x: f32, y: f32) -> Option<(usize, Part)> {
            let point = graphene::Point::new(x, y);
            let blocks = self.blocks.borrow();
            let index = blocks
                .iter()
                .position(|entry| self.area(entry).contains_point(&point))?;
            let area = self.area(&blocks[index]);
            // Very short blocks can only be moved; their time changes in the panel.
            let part = if area.height() < 3.0 * EDGE {
                Part::Body
            } else if y < area.y() + EDGE {
                Part::Start
            } else if y > area.y() + area.height() - EDGE {
                Part::End
            } else {
                Part::Body
            };
            Some((index, part))
        }

        /// Whether a new block can start at `minute`.
        pub fn is_free(&self, minute: u32) -> bool {
            minute < 24 * 60
                && !self
                    .blocks
                    .borrow()
                    .iter()
                    .any(|entry| (entry.span.0..entry.span.1).contains(&minute))
        }

        /// Marks `span`, growing the grid if it ends below it.
        pub fn set_pending(&self, span: Option<(u32, u32)>) {
            self.pending.set(span);
            let (first, last) = self.range.get();
            if let Some((_, end)) = span
                && end > last
            {
                self.range.set((first, end.div_ceil(60) * 60));
                self.obj().queue_resize();
            }
            self.obj().queue_draw();
        }

        /// Marks the span the drag going on gives with the pointer at `minute`.
        pub fn drag_to(&self, minute: u32) {
            let span = match self.drag.get() {
                Some(Drag::New { anchor }) => self.free_span(anchor, minute),
                Some(Drag::Block {
                    index,
                    part,
                    minute: grabbed,
                }) => self.changed_span(index, part, i64::from(minute) - i64::from(grabbed)),
                None => return,
            };
            self.set_pending(Some(span));
        }

        /// The span of the block `index` with `part` moved by `delta`
        /// minutes, rounded to whole slots, within the free time around it.
        fn changed_span(&self, index: usize, part: Part, delta: i64) -> (u32, u32) {
            let slot = i64::from(self.slot_minutes.get());
            let delta = (delta + slot / 2).div_euclid(slot) * slot;
            let (first, last) = self.range.get();
            let blocks = self.blocks.borrow();
            let (start, end) = blocks[index].span;
            let others = || {
                blocks
                    .iter()
                    .enumerate()
                    .filter(move |(other, _)| *other != index)
                    .map(|(_, entry)| entry.span)
            };
            let free_start = others()
                .map(|(_, end)| end)
                .filter(|&other_end| other_end <= start)
                .max()
                .unwrap_or(first)
                .min(start);
            let free_end = others()
                .map(|(start, _)| start)
                .filter(|&other_start| other_start >= end)
                .min()
                .unwrap_or(last)
                .max(end);
            let [start, end, free_start, free_end] =
                [start, end, free_start, free_end].map(i64::from);
            // A block starts on its day, so before midnight.
            let latest_start = 24 * 60 - 1;
            let shortest = slot.min(end - start);
            let (start, end) = match part {
                Part::Body => {
                    let length = end - start;
                    let start =
                        (start + delta).clamp(free_start, (free_end - length).min(latest_start));
                    (start, start + length)
                }
                Part::Start => (
                    (start + delta).clamp(free_start, (end - shortest).min(latest_start)),
                    end,
                ),
                Part::End => (start, (end + delta).clamp(start + shortest, free_end)),
            };
            let minute = |value: i64| u32::try_from(value).expect("spans stay within the grid");
            (minute(start), minute(end))
        }

        /// The whole slots from `anchor` to `minute`, at least one, cut to
        /// the free time around `anchor` and to the day.
        pub fn free_span(&self, anchor: u32, minute: u32) -> (u32, u32) {
            let slot = self.slot_minutes.get();
            let (first, _) = self.range.get();
            let (low, high) = (anchor.min(minute), anchor.max(minute));
            let start = low / slot * slot;
            let end = (high.div_ceil(slot) * slot).max(start + slot);
            let blocks = self.blocks.borrow();
            let free_start = blocks
                .iter()
                .map(|entry| entry.span.1)
                .filter(|&end| end <= anchor)
                .max()
                .unwrap_or(first);
            let free_end = blocks
                .iter()
                .map(|entry| entry.span.0)
                .filter(|&start| start > anchor)
                .min()
                .unwrap_or(24 * 60)
                .min(24 * 60);
            (start.max(free_start), end.min(free_end))
        }

        /// The lines of the slots and the labelled lines of full hours.
        fn snapshot_grid(&self, snapshot: &gtk::Snapshot, width: f32, foreground: &gdk::RGBA) {
            let (first, last) = self.range.get();
            let slot = self.slot_minutes.get();
            for minute in (first..=last).step_by(slot as usize) {
                let y = y_of(first, minute);
                let is_hour = minute % 60 == 0;
                let alpha = if is_hour { 0.2 } else { 0.07 };
                let line = graphene::Rect::new(LINE_X, y, width - LINE_X, 1.0);
                snapshot.append_color(&with_alpha(foreground, alpha), &line);
                if is_hour {
                    let text = format!("{:02}:00", minute / 60 % 24);
                    let layout = self.obj().create_pango_layout(Some(&text));
                    let (text_width, text_height) = layout.pixel_size();
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(
                        LABEL_WIDTH - text_width as f32,
                        y - text_height as f32 / 2.0,
                    ));
                    snapshot.append_layout(&layout, &with_alpha(foreground, 0.55));
                    snapshot.restore();
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

        for entry in imp.blocks.take() {
            entry.child.unparent();
        }
        let entries = day
            .blocks
            .iter()
            .map(|block| {
                let project = vault.project(&block.project);
                let child = block_content(block, vault.project_name(&block.project));
                child.set_parent(self);
                child.connect_has_focus_notify(glib::clone!(
                    #[weak(rename_to = timeline)]
                    self,
                    move |_| timeline.queue_draw()
                ));
                imp::Entry {
                    span: block.span(),
                    color: project.map(|project| {
                        gdk::RGBA::parse(project.color.as_str())
                            .expect("the core only accepts valid colours")
                    }),
                    child,
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
        popover.connect_closed(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |popover| {
                let imp = timeline.imp();
                imp.pending.set(None);
                imp.popover.take();
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

/// Title, project and first line of text of `block`.
fn block_content(block: &Block, project_name: &str) -> gtk::Widget {
    let label = |text: &str, classes: &[&str]| {
        gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(classes)
            .build()
    };
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if !block.title.is_empty() {
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
    let first_line = block
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty());
    if let Some(line) = first_line {
        content.append(&label(line, &["caption", "dim-label"]));
    }
    content.upcast()
}

/// Where `minute` lies below the top, for a grid that starts at `first`.
fn y_of(first: u32, minute: u32) -> f32 {
    PADDING + (minute - first) as f32 * MINUTE_HEIGHT
}

fn block_height(start: u32, end: u32) -> f32 {
    (end - start) as f32 * MINUTE_HEIGHT
}

fn append_knot(snapshot: &gtk::Snapshot, y: f32, radius: f32, color: &gdk::RGBA) {
    let bounds = graphene::Rect::new(LINE_X - radius, y - radius, 2.0 * radius, 2.0 * radius);
    snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, radius));
    snapshot.append_color(color, &bounds);
    snapshot.pop();
}

fn with_alpha(color: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    color.with_alpha(color.alpha() * alpha)
}
