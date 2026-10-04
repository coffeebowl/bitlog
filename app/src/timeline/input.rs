//! Pointer and keys: activating blocks, dragging them and their edges,
//! and selecting free time for new blocks.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::imp::{self, Drag, Part};
use super::{EDGE, MINUTE_HEIGHT, PADDING, Timeline};

impl Timeline {
    pub(super) fn setup_input(&self) {
        self.setup_click();
        self.setup_drag();
        self.setup_long_press();
        self.setup_cursor();
        self.setup_keys();
    }

    /// A click on a block activates it. Blocks do not take pointer input
    /// themselves: short ones are allocated taller than they are drawn.
    fn setup_click(&self) {
        let click = gtk::GestureClick::new();
        click.connect_released(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, _, x, y| {
                if let Some((index, _)) = timeline.imp().hit(x as f32, y as f32) {
                    timeline.imp().blocks.borrow()[index].child.grab_focus();
                    timeline.activate(index);
                }
            }
        ));
        self.add_controller(click);
    }

    /// Dragging over free time, or just clicking it, selects a span of
    /// whole slots for a new block. Dragging a block moves it, dragging its
    /// edges changes its start or end. On touch screens dragging scrolls
    /// instead.
    fn setup_drag(&self) {
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
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
            #[weak(rename_to = timeline)]
            self,
            move |drag, _, dy| {
                if let Some((_, start_y)) = drag.start_point() {
                    let imp = timeline.imp();
                    imp.drag_to(imp.minute_at((start_y + dy) as f32));
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
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
        self.add_controller(drag);
    }

    /// On touch screens a long press selects a slot of free time instead.
    fn setup_long_press(&self) {
        let long_press = gtk::GestureLongPress::builder().touch_only(true).build();
        long_press.connect_pressed(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, _, y| {
                let minute = timeline.imp().minute_at(y as f32);
                if let Some((start, end)) = timeline.select_free_span(minute) {
                    timeline.emit_by_name::<()>("span-selected", &[&start, &end]);
                }
            }
        ));
        self.add_controller(long_press);
    }

    /// The cursor shows whether a block is moved or its edge is changed.
    fn setup_cursor(&self) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, x, y| {
                let cursor = match timeline.imp().hit(x as f32, y as f32) {
                    Some((_, Part::Body)) => Some("grab"),
                    Some(_) => Some("ns-resize"),
                    None => None,
                };
                timeline.set_cursor_from_name(cursor);
            }
        ));
        self.add_controller(motion);
    }

    /// Enter and Space activate the block with the focus.
    fn setup_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
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
        self.add_controller(keys);
    }
}

impl imp::Timeline {
    /// The minute shown at the height `y`, within the grid.
    fn minute_at(&self, y: f32) -> u32 {
        let (first, last) = self.range.get();
        let offset = ((y - PADDING) / MINUTE_HEIGHT).max(0.0) as u32;
        (first + offset).min(last)
    }

    /// The block drawn at `x`, `y`, and where it is grabbed there.
    fn hit(&self, x: f32, y: f32) -> Option<(usize, Part)> {
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
    fn drag_to(&self, minute: u32) {
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
        let [start, end, free_start, free_end] = [start, end, free_start, free_end].map(i64::from);
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
}
