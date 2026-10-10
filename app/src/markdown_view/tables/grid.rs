//! Drawing tables: as grids while the cursor is elsewhere, and the dashes
//! of the delimiter row while it is in the table.

use std::ops::Range;

use bitlog_core::TableLine;
use gtk::prelude::*;
use gtk::{gdk, graphene, gsk, pango};

use super::super::decorations::{char_location, line_span};
use super::super::styling::Styling;
use super::super::{CODE_ALPHA, CORNER_RADIUS, GRID_ALPHA, MARKUP_ALPHA, tags};
use super::{Cell, ELLIPSIS, ROW_SPACING};
use crate::colors::with_alpha;
use crate::drawing::append_layout;

/// Hides the delimiter row of tables drawn as grids.
const DELIMITER_ROW_TAG: &str = "table-delimiter-row";
/// Shrinks the delimiter row to a thin line.
const DELIMITER_ROW_SCALE: f64 = 0.1;
/// Between the text and the edge of a table without outer pipes.
const CELL_PADDING: f32 = 6.0;

/// A table drawn as a grid.
#[derive(Debug)]
pub(in super::super) struct Grid {
    /// The header, the delimiter row and the body.
    lines: Vec<GridLine>,
    /// Where cut-off cells have an ellipsis, in characters, and how many
    /// pixels further right.
    ellipses: Vec<(i32, f32)>,
}

#[derive(Debug)]
struct GridLine {
    range: Range<i32>,
    /// The character offsets of the pipes between the cells.
    pipes: Vec<i32>,
    /// Whether a pipe opens the line, and whether one closes it.
    outer_pipes: (bool, bool),
}

impl Grid {
    /// Hides the pipes and delimiter row of the table of `lines`, which
    /// the grid draws instead, with the `ellipses` of cut-off cells, at
    /// byte offsets.
    pub(super) fn conceal(
        styling: &Styling,
        lines: &[TableLine],
        ellipses: &[(usize, f32)],
    ) -> Self {
        let (buffer, text) = (&styling.buffer, styling.text);
        let mut grid_lines = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let chars = styling.chars(&line.range);
            let (start, end) = (chars.start, chars.end);
            if index == 1 {
                // Last, so that it wins over the markup tag.
                tags::get_or_add(buffer, DELIMITER_ROW_TAG, || {
                    gtk::TextTag::builder()
                        .name(DELIMITER_ROW_TAG)
                        .foreground_rgba(&tags::INVISIBLE)
                        .scale(DELIMITER_ROW_SCALE)
                        .build()
                });
                // With its line break, which is as high as its font.
                let rest = &text[line.range.end..];
                let line_break = if rest.starts_with("\r\n") {
                    2
                } else {
                    i32::from(rest.starts_with('\n'))
                };
                tags::apply(buffer, DELIMITER_ROW_TAG, start..end + line_break);
            }
            let pipes: Vec<i32> = line
                .pipes
                .iter()
                .map(|&pipe| styling.offset(pipe))
                .collect();
            for &pipe in &pipes {
                tags::apply(buffer, tags::CONCEALED, pipe..pipe + 1);
            }
            let outer_pipes = (
                line.pipes
                    .first()
                    .is_some_and(|&pipe| text[line.range.start..pipe].trim().is_empty()),
                line.pipes
                    .last()
                    .is_some_and(|&pipe| text[pipe + 1..line.range.end].trim().is_empty()),
            );
            grid_lines.push(GridLine {
                range: start..end,
                pipes,
                outer_pipes,
            });
        }
        Self {
            lines: grid_lines,
            ellipses: ellipses
                .iter()
                .map(|&(at, shift)| (styling.offset(at), shift))
                .collect(),
        }
    }

    /// Draws the grid with a border, lines between its cells and a shaded
    /// header, in buffer coordinates.
    pub(in super::super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let (Some(first), Some(last)) = (self.lines.first(), self.lines.last()) else {
            return;
        };
        let Some((top, bottom)) =
            line_span(view, &(first.range.start..last.range.end + 1), visible)
        else {
            return;
        };
        // The space below the last row is on the line after it.
        let bottom = bottom + ROW_SPACING as f32;
        let buffer = view.buffer();
        let location = |offset: i32| char_location(view, offset);
        let center = |offset: i32| {
            let pipe = location(offset);
            (pipe.x() as f32 + pipe.width() as f32 / 2.0).round()
        };
        // The delimiter row is too thin to count.
        let rows = || {
            self.lines
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 1)
        };
        let (mut left, mut right) = (f32::MAX, f32::MIN);
        for (_, line) in rows() {
            let (opens, closes) = line.outer_pipes;
            left = left.min(match line.pipes.first() {
                Some(&pipe) if opens => center(pipe),
                _ => location(line.range.start).x() as f32 - CELL_PADDING,
            });
            right = right.max(match line.pipes.last() {
                Some(&pipe) if closes => center(pipe),
                _ => location(line.range.end).x() as f32 + CELL_PADDING,
            });
        }
        // The rows below the header start halfway into the space above
        // them, the rest is below the row before.
        let tops: Vec<f32> = self
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let (y, _) = view.line_yrange(&buffer.iter_at_offset(line.range.start));
                let space = if index == 0 { 0 } else { ROW_SPACING };
                (y + space) as f32
            })
            .collect();
        // Each row reaches down to the next, over the delimiter row.
        let row_bottom = |index: usize| {
            let next = if index == 0 { 2 } else { index + 1 };
            tops.get(next).copied().unwrap_or(bottom)
        };
        // A line of a single pipe has no inner ones.
        let inner = |line: &GridLine| {
            let (opens, closes) = line.outer_pipes;
            let end = line.pipes.len().saturating_sub(usize::from(closes));
            line.pipes
                .get(usize::from(opens)..end)
                .unwrap_or_default()
                .to_vec()
        };
        // The lines between the columns are where the first row has them:
        // the others may be a pixel off, as their spacing is rounded.
        let mut columns: Vec<f32> = Vec::new();
        for (_, line) in rows() {
            for pipe in inner(line).into_iter().skip(columns.len()) {
                columns.push(center(pipe));
            }
        }
        let color = view.color();
        let line_color = with_alpha(&color, GRID_ALPHA);
        let bounds = graphene::Rect::new(left, top, right - left, bottom - top);
        let rounded = gsk::RoundedRect::from_rect(bounds, CORNER_RADIUS);
        snapshot.push_rounded_clip(&rounded);
        let header = graphene::Rect::new(left, top, right - left, row_bottom(0) - top);
        snapshot.append_color(&with_alpha(&color, CODE_ALPHA), &header);
        for (index, line) in rows() {
            let row_top = tops[index];
            if index > 0 {
                let rule = graphene::Rect::new(left, row_top, right - left, 1.0);
                snapshot.append_color(&line_color, &rule);
            }
            for x in columns.iter().take(inner(line).len()) {
                let height = row_bottom(index) - row_top;
                let bounds = graphene::Rect::new(*x, row_top, 1.0, height);
                snapshot.append_color(&line_color, &bounds);
            }
        }
        snapshot.pop();
        snapshot.append_border(&rounded, &[1.0; 4], &[line_color; 4]);
        for &(at, shift) in &self.ellipses {
            let location = location(at);
            let layout = view.create_pango_layout(Some(ELLIPSIS));
            let start = (
                location.x() as f32 + shift,
                location.y() as f32 + location.height() as f32 / 2.0,
            );
            append_layout(snapshot, &layout, start, (0.0, 0.5), &color);
        }
    }
}

/// The delimiter row of a table the cursor is in, with dashes drawn across
/// its cells instead of its own, as if it were filled already.
#[derive(Debug)]
pub(in super::super) struct DelimiterDashes {
    /// Where the row starts, in characters.
    start: i32,
    /// The dashes of each cell, in characters.
    cells: Vec<Range<i32>>,
    /// Where the dashes of each cell go, see `Cells::dash_spans`, unless
    /// the columns are not lined up.
    spans: Option<Vec<(i32, i32)>>,
}

impl DelimiterDashes {
    /// Hides the dashes of the delimiter row `cells` of `line` but the last,
    /// as the cells were measured, and conceals the last, which keeps a
    /// cell from being empty and is spaced out to line it up.
    pub(super) fn conceal(
        styling: &Styling,
        line: &TableLine,
        cells: &[Cell],
        spans: Option<Vec<(i32, i32)>>,
    ) -> Self {
        let mut dashes = Vec::new();
        for cell in cells {
            for part in &cell.hidden {
                styling.tag(tags::HIDDEN, part);
                styling.tag(tags::CONCEALED, &(part.end..part.end + 1));
                dashes.push(styling.chars(&(part.start..part.end + 1)));
            }
        }
        Self {
            start: styling.offset(line.range.start),
            cells: dashes,
            spans,
        }
    }

    /// Draws as many dashes as fit where those of each cell go, in buffer
    /// coordinates.
    pub(in super::super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let color = with_alpha(&view.color(), MARKUP_ALPHA);
        let buffer = view.buffer();
        let dash_width = monospace_layout(view, "-").size().0 as f32;
        // Between the characters: a pipe at the end of a line has no
        // height as a character.
        let location = |offset: i32| {
            let iter = buffer.iter_at_offset(offset);
            view.cursor_locations(Some(&iter)).0
        };
        let line_start = location(self.start).x() as f32;
        let scale = pango::SCALE as f32;
        for (index, cell) in self.cells.iter().enumerate() {
            // From the concealed dash, as those before it are hidden.
            let (start, end) = (location(cell.end - 1), location(cell.end));
            let out_of_sight = start.y() + start.height() < visible.y()
                || start.y() > visible.y() + visible.height();
            // Not across a wrapped line.
            if out_of_sight || dash_width <= 0.0 || start.y() != end.y() {
                continue;
            }
            let span = self.spans.as_ref().and_then(|spans| spans.get(index));
            let (x, width) = match span {
                Some(&(from, to)) => (line_start + from as f32 / scale, (to - from) as f32),
                None => (start.x() as f32, (end.x() - start.x()) as f32 * scale),
            };
            let count = (width / dash_width).round() as usize;
            let layout = monospace_layout(view, &"-".repeat(count.max(1)));
            // On the baseline of the text: laid out as high as its line,
            // from the top of it. Wrapped rows only get the middle of theirs.
            let (top, height) = view.line_yrange(&buffer.iter_at_offset(cell.start));
            let layout_height = layout.pixel_size().1;
            let y = if height <= layout_height + 1 {
                top as f32
            } else {
                start.y() as f32 + (start.height() - layout_height) as f32 / 2.0
            };
            append_layout(snapshot, &layout, (x, y), (0.0, 0.0), &color);
        }
    }
}

/// `text` in the monospace font of tables being edited, with the line
/// height of the text.
fn monospace_layout(view: &gtk::TextView, text: &str) -> pango::Layout {
    let layout = view.create_pango_layout(Some(text));
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrString::new_family("monospace"));
    attributes.insert(pango::AttrFloat::new_line_height(f64::from(
        tags::LINE_HEIGHT,
    )));
    layout.set_attributes(Some(&attributes));
    layout
}
