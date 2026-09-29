//! Tables: their columns line up without changing the text, and while the
//! cursor is elsewhere they are drawn as grids, without their pipes and
//! delimiter row.

use std::ops::Range;

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk, pango};
use knotbook_core::{Formatting, MarkdownStyle, TableLine};

use super::decorations::{Decorations, char_location, line_span};
use super::styling::Styling;
use super::{CODE_ALPHA, CORNER_RADIUS, GRID_ALPHA, tags};
use crate::colors::with_alpha;

const HEADER_ROW_TAG: &str = "table-header-row";
const ROW_TAG: &str = "table-row";
/// Makes room below the last row of a table, on the line after it.
const AFTER_TAG: &str = "table-after";
/// Hides the delimiter row of tables drawn as grids.
const DELIMITER_ROW_TAG: &str = "table-delimiter-row";
/// Space above and below each row. Rows only have space above them, the
/// one below a row is above the next: below lines with hidden markup, GTK
/// takes the pointer for a place off the end of the line and aborts.
const ROW_SPACING: i32 = 4;
/// Shrinks the delimiter row to a thin line.
const DELIMITER_ROW_SCALE: f64 = 0.1;
/// Between the text and the edge of a table without outer pipes.
const CELL_PADDING: f32 = 6.0;

/// Styles the tables of `formatting`, and adds the grids of those the
/// cursor is not in to `decorations`.
pub(super) fn style(styling: &Styling, formatting: &Formatting, decorations: &mut Decorations) {
    let spacing_tag = |name: &str, pixels: i32| {
        tags::get_or_add(&styling.buffer, name, || {
            gtk::TextTag::builder()
                .name(name)
                .pixels_above_lines(pixels)
                .build()
        })
    };
    let header_row = spacing_tag(HEADER_ROW_TAG, ROW_SPACING);
    let row = spacing_tag(ROW_TAG, 2 * ROW_SPACING);
    spacing_tag(AFTER_TAG, ROW_SPACING);
    for lines in &formatting.tables {
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            continue;
        };
        let range = first.range.start..last.range.end;
        decorations.revealable.push(styling.chars(&range));
        align_columns(styling, lines, &formatting.styles, &decorations.hidden);
        for (index, line) in lines.iter().enumerate() {
            let tag = match index {
                0 => &header_row,
                1 => continue,
                _ => &row,
            };
            let chars = styling.chars(&line.range);
            let start = styling.buffer.iter_at_offset(chars.start);
            let end = styling.buffer.iter_at_offset(chars.end);
            styling.buffer.apply_tag(tag, &start, &end);
        }
        let mut after = styling
            .buffer
            .iter_at_offset(styling.offset(last.range.end));
        if after.forward_line() {
            tags::apply_to_lines(&styling.buffer, AFTER_TAG, after.offset()..after.offset());
        }
        if !styling.is_at(&range) {
            decorations.grids.push(Grid::conceal(styling, lines));
        }
    }
}

/// Lines up the columns of a table: the last character of each cell,
/// usually a space, is spaced out to the width of the widest cell of its
/// column.
fn align_columns(
    styling: &Styling,
    lines: &[TableLine],
    styles: &[(Range<usize>, MarkdownStyle)],
    hidden: &[Range<usize>],
) {
    let widths: Vec<Vec<i32>> = lines
        .iter()
        .map(|line| {
            line.cells
                .iter()
                .map(|cell| text_width(styling, cell, styles, hidden))
                .collect()
        })
        .collect();
    let mut column_widths: Vec<i32> = Vec::new();
    for line_widths in &widths {
        for (column, width) in line_widths.iter().enumerate() {
            match column_widths.get_mut(column) {
                Some(widest) => *widest = (*widest).max(*width),
                None => column_widths.push(*width),
            }
        }
    }
    for (line, line_widths) in lines.iter().zip(&widths) {
        let cells = line.cells.iter().zip(line_widths).zip(&column_widths);
        for ((cell, width), widest) in cells {
            let pixels = (widest - width + pango::SCALE / 2) / pango::SCALE;
            if pixels == 0 || cell.is_empty() {
                continue;
            }
            let end = styling.offset(cell.end);
            let spacing = tags::spacing(&styling.buffer, pixels);
            tags::apply(&styling.buffer, &spacing, end - 1..end);
        }
    }
}

/// How wide `range` of the text is shown, in Pango units, with the
/// `styles` that change widths and without the `hidden` markup, which is
/// sorted.
fn text_width(
    styling: &Styling,
    range: &Range<usize>,
    styles: &[(Range<usize>, MarkdownStyle)],
    hidden: &[Range<usize>],
) -> i32 {
    let text = styling.text;
    let hidden: Vec<Range<usize>> = hidden
        .iter()
        .map(|part| part.start.max(range.start)..part.end.min(range.end))
        .filter(|part| !part.is_empty())
        .collect();
    let mut shown = String::new();
    let mut start = range.start;
    for part in &hidden {
        shown.push_str(&text[start..part.start]);
        start = part.end;
    }
    shown.push_str(&text[start..range.end]);
    let layout = styling.layout(&shown);
    let attributes = pango::AttrList::new();
    // Where `offset` of the text is in what is shown.
    let index = |offset: usize| {
        let hidden_before: usize = hidden
            .iter()
            .filter(|part| part.start < offset)
            .map(|part| part.end.min(offset) - part.start)
            .sum();
        u32::try_from(offset - range.start - hidden_before).unwrap_or(u32::MAX)
    };
    for (style_range, style) in styles {
        let start = style_range.start.max(range.start);
        let end = style_range.end.min(range.end);
        if start >= end {
            continue;
        }
        let mut attribute: pango::Attribute = match style {
            MarkdownStyle::Strong | MarkdownStyle::TableHeader => {
                pango::AttrInt::new_weight(pango::Weight::Bold).into()
            }
            MarkdownStyle::Emphasis => pango::AttrInt::new_style(pango::Style::Italic).into(),
            MarkdownStyle::Code => pango::AttrString::new_family("monospace").into(),
            _ => continue,
        };
        attribute.set_start_index(index(start));
        attribute.set_end_index(index(end));
        attributes.insert(attribute);
    }
    layout.set_attributes(Some(&attributes));
    layout.size().0
}

/// A table drawn as a grid.
#[derive(Debug)]
pub(super) struct Grid {
    /// The header, the delimiter row and the body.
    lines: Vec<GridLine>,
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
    /// the grid draws instead.
    fn conceal(styling: &Styling, lines: &[TableLine]) -> Self {
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
        Self { lines: grid_lines }
    }

    /// Draws the grid with a border, lines between its cells and a shaded
    /// header, in buffer coordinates.
    pub(super) fn snapshot(
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
            let (opens, closes) = line.outer_pipes;
            // A line of a single pipe has no inner ones.
            let inner = line
                .pipes
                .get(usize::from(opens)..line.pipes.len() - usize::from(closes))
                .unwrap_or_default();
            for &pipe in inner {
                let height = row_bottom(index) - row_top;
                let bounds = graphene::Rect::new(center(pipe), row_top, 1.0, height);
                snapshot.append_color(&line_color, &bounds);
            }
        }
        snapshot.pop();
        snapshot.append_border(&rounded, &[1.0; 4], &[line_color; 4]);
    }
}
