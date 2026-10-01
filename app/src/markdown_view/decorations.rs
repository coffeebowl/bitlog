//! What is drawn beside the text: bars beside quotes, bullets over the
//! markers of list items and lines for rules, and what the other modules
//! add, like the cards of code blocks, diagrams and the grids of tables.
//! And the syntax that is hidden until the cursor is at it.

use std::ops::Range;

use bitlog_core::{Formatting, MarkdownStyle};
use gtk::prelude::*;
use gtk::{gdk, graphene, gsk, pango};

use super::callouts::CalloutCard;
use super::check_boxes::CheckBox;
use super::code_blocks::CodeCard;
use super::diagrams::DiagramCard;
use super::styling::{Styling, holds};
use super::tables::Grid;
use super::{GRID_ALPHA, MARKUP_ALPHA, lists, tags};
use crate::colors::with_alpha;

pub(super) const QUOTE_BAR_WIDTH: f32 = 3.0;
/// The bullets of lists, by depth, over again for deeper ones.
const BULLETS: [&str; 3] = ["•", "◦", "▪"];
const BULLET_ALPHA: f32 = 0.7;
/// Bullets are a little larger than the text.
const BULLET_SCALE: f64 = 1.4;

/// As of the text last styled, in characters.
#[derive(Debug, Default)]
pub(super) struct Decorations {
    /// Quotes with a bar, but for callouts.
    pub(super) quotes: Vec<Range<i32>>,
    /// Where the marker of a bullet list item is, and how deep the list is.
    pub(super) bullets: Vec<(i32, u8)>,
    /// Rules drawn as lines, not those being edited.
    rules: Vec<Range<i32>>,
    pub(super) code_blocks: Vec<CodeCard>,
    /// Code blocks drawn as diagrams, not those being edited.
    pub(super) diagrams: Vec<DiagramCard>,
    pub(super) callouts: Vec<CalloutCard>,
    /// Tables drawn as grids, not those being edited.
    pub(super) grids: Vec<Grid>,
    pub(super) check_boxes: Vec<CheckBox>,
    /// Tables, rules, code blocks, the markers of callouts, the check
    /// boxes of tasks and inline elements, which show their Markdown while
    /// the cursor is at them, without the last line break.
    pub(super) revealable: Vec<Range<i32>>,
    /// The hidden markup of inline elements, as sorted byte ranges.
    pub(super) hidden: Vec<Range<usize>>,
}

impl Decorations {
    /// Collects the quotes, bullets and rules of `formatting`, and hides
    /// the rules it draws and the markup of inline elements, but for those
    /// the cursor is at.
    pub(super) fn collect(styling: &Styling, formatting: &Formatting) -> Self {
        let mut decorations = Self::default();
        for inline in &formatting.inline_markup {
            decorations.revealable.push(styling.chars(&inline.element));
            if styling.is_at(&inline.element) {
                continue;
            }
            for markup in &inline.markup {
                styling.tag(tags::HIDDEN, markup);
                decorations.hidden.push(markup.clone());
            }
        }
        decorations.hidden.sort_by_key(|range| range.start);
        for (range, style) in &formatting.styles {
            match style {
                MarkdownStyle::Quote => decorations.quotes.push(styling.chars(range)),
                MarkdownStyle::Bullet(depth) => {
                    decorations
                        .bullets
                        .push((styling.offset(range.start), *depth));
                }
                MarkdownStyle::Rule => {
                    decorations.revealable.push(styling.chars(range));
                    if !styling.is_at(range) {
                        styling.tag(tags::CONCEALED, range);
                        decorations.rules.push(styling.chars(range));
                    }
                }
                _ => {}
            }
        }
        decorations
    }

    /// The starts of what the `cursor` is at and shows its Markdown.
    pub(super) fn revealed(&self, cursor: Option<i32>) -> Vec<i32> {
        self.revealable
            .iter()
            .filter(|range| holds(range, cursor))
            .map(|range| range.start)
            .collect()
    }

    /// Whether a diagram has another height in `view` than it has room
    /// for, as the view changed its width.
    pub(super) fn misfit(&self, view: &gtk::TextView) -> bool {
        self.diagrams.iter().any(|card| card.misfits(view))
    }

    /// The check box at `x`, `y` in `view`, in buffer coordinates.
    pub(super) fn check_box_at(&self, view: &gtk::TextView, x: i32, y: i32) -> Option<&CheckBox> {
        self.check_boxes
            .iter()
            .find(|check_box| check_box.contains(view, x, y))
    }

    /// Draws what goes below the text, in buffer coordinates.
    pub(super) fn snapshot_below(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot) {
        let visible = view.visible_rect();
        let color = view.color();
        let (left, right) = text_edges(view, &visible);
        for card in &self.code_blocks {
            card.snapshot(view, snapshot, &visible);
        }
        for card in &self.diagrams {
            card.snapshot(view, snapshot, &visible);
        }
        for range in &self.quotes {
            if let Some((top, bottom)) = line_span(view, range, &visible) {
                let bounds = graphene::Rect::new(left, top, QUOTE_BAR_WIDTH, bottom - top);
                let bar = gsk::RoundedRect::from_rect(bounds, QUOTE_BAR_WIDTH / 2.0);
                snapshot.push_rounded_clip(&bar);
                snapshot.append_color(&with_alpha(&color, MARKUP_ALPHA), &bounds);
                snapshot.pop();
            }
        }
        for callout in &self.callouts {
            callout.snapshot(view, snapshot, &visible);
        }
        for range in &self.rules {
            if let Some((top, bottom)) = line_span(view, range, &visible) {
                let y = ((top + bottom) / 2.0).round();
                let bounds = graphene::Rect::new(left, y, right - left, 1.0);
                snapshot.append_color(&with_alpha(&color, GRID_ALPHA), &bounds);
            }
        }
        for grid in &self.grids {
            grid.snapshot(view, snapshot, &visible);
        }
        for check_box in &self.check_boxes {
            check_box.snapshot(view, snapshot);
        }
    }

    /// Draws bullets over the hidden markers of bullet list items, in buffer
    /// coordinates.
    pub(super) fn snapshot_above(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot) {
        let visible = view.visible_rect();
        // In the middle of the room of markers.
        let markers = (lists::MARKER_WIDTH - lists::MARKER_GAP) as f32;
        let color = with_alpha(&view.color(), BULLET_ALPHA);
        for &(offset, depth) in &self.bullets {
            let marker = char_location(view, offset);
            if marker.y() + marker.height() < visible.y()
                || marker.y() > visible.y() + visible.height()
            {
                continue;
            }
            let bullet = BULLETS[usize::from(depth.max(1) - 1) % BULLETS.len()];
            let layout = view.create_pango_layout(Some(bullet));
            let attributes = pango::AttrList::new();
            attributes.insert(pango::AttrFloat::new_scale(BULLET_SCALE));
            layout.set_attributes(Some(&attributes));
            let (width, height) = layout.pixel_size();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                marker.x() as f32 + (markers - width as f32) / 2.0,
                marker.y() as f32 + (marker.height() - height) as f32 / 2.0,
            ));
            snapshot.append_layout(&layout, &color);
            snapshot.restore();
        }
    }
}

/// Where the character at `offset` is in `view`, in buffer coordinates.
/// `TextView::iter_location` counts invisible text before it on its line,
/// as if it were visible, which the cursor positions around it do not.
pub(super) fn char_location(view: &gtk::TextView, offset: i32) -> gdk::Rectangle {
    let buffer = view.buffer();
    let iter = buffer.iter_at_offset(offset);
    let mut next = iter;
    next.forward_char();
    let (start, _) = view.cursor_locations(Some(&iter));
    let (end, _) = view.cursor_locations(Some(&next));
    // Only its line tells how high it is.
    let line = view.iter_location(&iter);
    let width = if end.y() == start.y() {
        end.x() - start.x()
    } else {
        line.width()
    };
    gdk::Rectangle::new(start.x(), line.y(), width, line.height())
}

/// The top and bottom of the lines of `range` in `view`, in buffer
/// coordinates, unless they are out of sight.
pub(super) fn line_span(
    view: &gtk::TextView,
    range: &Range<i32>,
    visible: &gdk::Rectangle,
) -> Option<(f32, f32)> {
    let buffer = view.buffer();
    let (top, _) = view.line_yrange(&buffer.iter_at_offset(range.start));
    // Ranges may end with the line break of their last line.
    let last = buffer.iter_at_offset((range.end - 1).max(range.start));
    let (y, height) = view.line_yrange(&last);
    let bottom = y + height;
    (bottom >= visible.y() && top <= visible.y() + visible.height())
        .then_some((top as f32, bottom as f32))
}

/// Where the text of `view` starts and ends across, in buffer coordinates,
/// as much of it as is `visible`.
pub(super) fn text_edges(view: &gtk::TextView, visible: &gdk::Rectangle) -> (f32, f32) {
    let left = view.left_margin() as f32;
    let right = (visible.x() + visible.width() - view.right_margin()) as f32;
    (left, right)
}
