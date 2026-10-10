//! What is drawn in place of parts of the text, like diagrams and images:
//! made in the background, kept while the text has it, and drawn centred in
//! the text, at most nine tenths as wide.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};

use super::CORNER_RADIUS;
use super::decorations::text_edges;

/// Drawings are drawn at most this share of the width of the text, which
/// reads calmer when scrolling.
const WIDTH_SHARE: f32 = 0.9;

/// A texture to draw, with how large it is meant to be.
#[derive(Debug)]
pub(super) struct Drawing {
    texture: gdk::Texture,
    /// In pixels of the view.
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
pub(super) enum State {
    Pending,
    Failed,
    Done(Rc<Drawing>),
}

/// The drawings of the text as last formatted, by what they are made of.
#[derive(Debug)]
pub(super) struct Drawings<R>(RefCell<HashMap<R, State>>);

/// One formatting of the text: the drawings it has are kept, the others
/// are dropped once it ends.
pub(super) struct Pass<'a, R> {
    drawings: &'a Drawings<R>,
    known: HashMap<R, State>,
    kept: HashMap<R, State>,
    missing: Vec<R>,
}

/// Where a line starts, kept by a mark, as the view may draw after a change
/// before the text is formatted again. The mark goes with it.
#[derive(Debug)]
pub(super) struct LineMark(gtk::TextMark);

impl Drawing {
    pub(super) fn new(texture: gdk::Texture, width: f32, height: f32) -> Self {
        Self {
            texture,
            width,
            height,
        }
    }

    /// How wide and high it is drawn in `view`: no larger than it is, and
    /// no wider than its share of the text.
    pub(super) fn fit(&self, view: &gtk::TextView) -> (f32, f32) {
        let (left, right) = text_edges(view, &view.visible_rect());
        let available = (right - left) * WIDTH_SHARE;
        // Before the view has a size, it has no room.
        let scale = if available > 0.0 {
            (available / self.width).min(1.0)
        } else {
            1.0
        };
        (self.width * scale, self.height * scale)
    }

    /// How high it is drawn in `view`, in whole pixels.
    pub(super) fn height(&self, view: &gtk::TextView) -> i32 {
        self.fit(view).1.round() as i32
    }

    /// Where it is drawn in `view` from `top` down, centred in the text, in
    /// buffer coordinates, unless it is out of the `visible` part.
    pub(super) fn bounds(
        &self,
        view: &gtk::TextView,
        top: f32,
        visible: &gdk::Rectangle,
    ) -> Option<graphene::Rect> {
        let (left, right) = text_edges(view, visible);
        let (width, height) = self.fit(view);
        let x = left + ((right - left - width) / 2.0).round();
        let shown =
            top + height >= visible.y() as f32 && top <= (visible.y() + visible.height()) as f32;
        shown.then(|| graphene::Rect::new(x, top, width, height))
    }

    /// Draws it within `bounds`, with round corners.
    pub(super) fn snapshot(&self, snapshot: &gtk::Snapshot, bounds: &graphene::Rect) {
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(*bounds, CORNER_RADIUS));
        snapshot.append_texture(&self.texture, bounds);
        snapshot.pop();
    }
}

impl<R> Default for Drawings<R> {
    fn default() -> Self {
        Self(RefCell::default())
    }
}

impl<R: Clone + Eq + Hash> Drawings<R> {
    /// Starts formatting the text again.
    pub(super) fn pass(&self) -> Pass<'_, R> {
        Pass {
            drawings: self,
            known: self.0.take(),
            kept: HashMap::new(),
            missing: Vec::new(),
        }
    }

    /// Keeps the `drawing` made for `request`, or that it cannot be made.
    /// Returns whether the text still has it.
    pub(super) fn finish(&self, request: R, drawing: Option<Drawing>) -> bool {
        let mut states = self.0.borrow_mut();
        let Some(state @ State::Pending) = states.get_mut(&request) else {
            return false;
        };
        *state = match drawing {
            Some(drawing) => State::Done(Rc::new(drawing)),
            None => State::Failed,
        };
        true
    }
}

impl<R: Clone + Eq + Hash> Pass<'_, R> {
    /// How the drawing for `request` is, which the text has. One not known
    /// yet is pending and to be made.
    pub(super) fn state(&mut self, request: R) -> State {
        let Self {
            known,
            kept,
            missing,
            ..
        } = self;
        kept.entry(request.clone())
            .or_insert_with(|| {
                known.remove(&request).unwrap_or_else(|| {
                    missing.push(request);
                    State::Pending
                })
            })
            .clone()
    }

    /// Keeps the drawing for `request` without making it, as for a diagram
    /// being edited, in case it is left as it was.
    pub(super) fn keep(&mut self, request: R) {
        if let Some(state) = self.known.remove(&request) {
            self.kept.insert(request, state);
        }
    }

    /// Ends the formatting. Returns the drawings to make, which
    /// `Drawings::finish` takes when they are made.
    pub(super) fn end(self) -> Vec<R> {
        self.drawings.0.replace(self.kept);
        self.missing
    }
}

impl LineMark {
    /// At the start of the line with the character at `offset` of `buffer`.
    pub(super) fn new(buffer: &gtk::TextBuffer, offset: i32) -> Self {
        let mut iter = buffer.iter_at_offset(offset);
        iter.set_line_offset(0);
        Self(buffer.create_mark(None, &iter, true))
    }

    /// The top and bottom of the line in `view`, in buffer coordinates.
    pub(super) fn line_span(&self, view: &gtk::TextView) -> (f32, f32) {
        let (top, height) = view.line_yrange(&view.buffer().iter_at_mark(&self.0));
        (top as f32, (top + height) as f32)
    }
}

impl Drop for LineMark {
    fn drop(&mut self) {
        if let Some(buffer) = self.0.buffer() {
            buffer.delete_mark(&self.0);
        }
    }
}
