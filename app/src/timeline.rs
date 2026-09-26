use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveTime, Timelike};
use glib::subclass::Signal;
use gtk::{gdk, glib, graphene, gsk, pango};
use knotbook_core::{Block, Day, Vault};

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
                // Emitted with the index of the block in the day.
                vec![
                    Signal::builder("block-activated")
                        .param_types([u32::static_type()])
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
                    let point = graphene::Point::new(x as f32, y as f32);
                    let hit = timeline
                        .imp()
                        .blocks
                        .borrow()
                        .iter()
                        .position(|entry| timeline.imp().area(entry).contains_point(&point));
                    if let Some(index) = hit {
                        timeline.imp().blocks.borrow()[index].child.grab_focus();
                        timeline.activate(index);
                    }
                }
            ));
            timeline.add_controller(click);

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
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let width = widget.width() as f32;
            let foreground = widget.color();
            let (first, last) = self.range.get();

            self.snapshot_grid(snapshot, width, &foreground);
            let line = graphene::Rect::new(LINE_X - 1.0, 0.0, 2.0, y_of(first, last) + PADDING);
            snapshot.append_color(&with_alpha(&foreground, 0.3), &line);

            let now = self.is_today.get().then(|| minutes(Local::now().time()));
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
        }
    }

    impl Timeline {
        /// Where `entry` is drawn.
        pub fn area(&self, entry: &Entry) -> graphene::Rect {
            let (first, _) = self.range.get();
            let (start, end) = entry.span;
            graphene::Rect::new(
                BLOCK_X,
                y_of(first, start) + 1.0,
                self.obj().width() as f32 - BLOCK_X,
                block_height(start, end) - 2.0,
            )
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
            .chain([minutes(grid.day_start)])
            .min()
            .expect("the grid start is always there");
        let last = spans
            .map(|(_, end)| end)
            .chain([minutes(grid.day_end)])
            .max()
            .expect("the grid end is always there");
        imp.range.set((first / 60 * 60, last.div_ceil(60) * 60));
        imp.slot_minutes.set(grid.slot_minutes);
        imp.is_today.set(is_today);
        imp.selected.set(None);

        for entry in imp.blocks.take() {
            entry.child.unparent();
        }
        let entries = day
            .blocks
            .iter()
            .map(|block| {
                let project = vault.project(&block.project);
                let project_name =
                    project.map_or_else(|| block.project.to_string(), |p| p.name.clone());
                let child = block_content(block, &project_name);
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

fn minutes(time: NaiveTime) -> u32 {
    time.hour() * 60 + time.minute()
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
