//! Tables: their columns line up without changing the text, and while the
//! cursor is elsewhere they are drawn as grids, without their pipes and
//! delimiter row. Cells too wide for the view are cut off then, with an
//! ellipsis; while the cursor is in such a table, its columns do not line
//! up, so that its rows wrap like other text. While the cursor is in a
//! table, it is in a monospace font, and the dashes of its delimiter row
//! are drawn across the cells, but for while the cursor is in that row.
//! Leaving the table tidies up its text: spaces between cells and pipes,
//! as the last character of a cell is spaced out to line it up, and the
//! delimiter row filled with dashes.

mod grid;

use std::ops::Range;

use bitlog_core::{Formatting, MarkdownStyle, TableLine, tidied_table};
use gtk::glib::translate::IntoGlib;
use gtk::pango;
use gtk::prelude::*;

pub(super) use self::grid::{DelimiterDashes, Grid};
use super::decorations::{Decorations, text_edges};
use super::styling::Styling;
use super::tags;

const HEADER_ROW_TAG: &str = "table-header-row";
/// The tables the cursor is in.
const EDITED_TAG: &str = "table-edited";
/// Pipes, which are never bold, so that they are as wide in the header.
const PIPE_TAG: &str = "table-pipe";
const ROW_TAG: &str = "table-row";
/// Makes room below the last row of a table, on the line after it.
const AFTER_TAG: &str = "table-after";
/// Space above and below each row. Rows only have space above them, the
/// one below a row is above the next: below lines with hidden markup, GTK
/// takes the pointer for a place off the end of the line and aborts.
const ROW_SPACING: i32 = 4;
/// Of the width of the view, in pixels: the widths of cells are rounded.
const SLACK: i32 = 4;
/// What cut-off cells end with, or URLs have in their middle.
const ELLIPSIS: &str = "…";
/// As wide as the shortest a cell is cut to.
const SHORTEST_CUT: &str = "mmmm…";
/// A cut-off URL keeps its end, as a share of what fits: one in this many.
const URL_END_SHARE: i32 = 3;
/// What columns are lined up to, in Pango units: an eighth of a pixel, as
/// text is placed by fractions of pixels, but every width takes a tag.
const SPACING_STEP: i32 = pango::SCALE / 8;

/// Whether a table fits the width of the view, as when it was styled.
#[derive(Debug)]
pub(super) struct Fit {
    /// How wide its widest row is with the columns lined up, in pixels.
    natural: i32,
    /// The width of the text, less the slack, in pixels.
    room: i32,
}

impl Fit {
    fn fits(natural: i32, room: i32) -> bool {
        // Before the view has a width, any table fits.
        room <= 0 || natural <= room
    }

    /// Whether the table fits the width of `view` otherwise than when it
    /// was styled, or was cut off for another width.
    pub(super) fn misfits(&self, view: &gtk::TextView) -> bool {
        let room = room(view);
        room != self.room
            && !(Self::fits(self.natural, self.room) && Self::fits(self.natural, room))
    }
}

/// The width of the text of `view` for tables, in pixels.
fn room(view: &gtk::TextView) -> i32 {
    let (left, right) = text_edges(view, &view.visible_rect());
    (right - left) as i32 - SLACK
}

/// What changes how wide the text of a table is shown.
#[derive(Clone, Copy)]
struct Markup<'a> {
    /// Among others, the styles that change widths.
    styles: &'a [(Range<usize>, MarkdownStyle)],
    /// The hidden markup of inline elements, sorted.
    hidden: &'a [Range<usize>],
    /// Whether the table is in a monospace font.
    monospace: bool,
}

/// A table tidied up, see `tidied_table`.
#[derive(Debug, Clone)]
pub(super) struct TidiedTable {
    /// The characters of the table, and its text.
    pub(super) table: (Range<i32>, String),
    pub(super) tidied: String,
}

/// Styles the tables of `formatting`, and adds to `decorations` the grids
/// of those the cursor is not in, the delimiter dashes of the one it is in,
/// how they fit and how to tidy them up.
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
    tags::get_or_add(&styling.buffer, EDITED_TAG, || {
        gtk::TextTag::builder()
            .name(EDITED_TAG)
            .family("monospace")
            .build()
    });
    tags::get_or_add(&styling.buffer, PIPE_TAG, || {
        gtk::TextTag::builder()
            .name(PIPE_TAG)
            .weight(pango::Weight::Normal.into_glib())
            .build()
    });
    let room = room(styling.view);
    let hidden = decorations.hidden.clone();
    let markup = Markup {
        styles: &formatting.styles,
        hidden: &hidden,
        monospace: false,
    };
    for lines in &formatting.tables {
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            continue;
        };
        let range = first.range.start..last.range.end;
        decorations.revealable.push(styling.chars(&range));
        // Shows its dashes while the cursor is in it.
        if let Some(delimiter_row) = lines.get(1) {
            decorations
                .revealable
                .push(styling.chars(&delimiter_row.range));
        }
        if let Some((range, tidied)) = tidied_table(styling.text, lines) {
            decorations.tidied_tables.push(TidiedTable {
                table: (styling.chars(&range), styling.text[range].to_owned()),
                tidied,
            });
        }
        let shown = styling.is_at(&range);
        if shown {
            styling.tag(EDITED_TAG, &range);
        }
        for &pipe in lines.iter().flat_map(|line| &line.pipes) {
            styling.tag(PIPE_TAG, &(pipe..pipe + 1));
        }
        let markup = Markup {
            monospace: shown,
            ..markup
        };
        let ghost_dashes = shown && lines.get(1).is_some_and(|line| !styling.is_at(&line.range));
        let mut cells = Cells::measure(styling, lines, ghost_dashes, markup);
        let natural = cells.natural_width(styling, markup);
        let fits = Fit::fits(natural, room);
        decorations.table_fits.push(Fit { natural, room });
        let ellipses = if shown || fits {
            Vec::new()
        } else {
            cells.cut(styling, room, markup)
        };
        let lined_up = !shown || fits;
        if lined_up {
            cells.line_up(styling, shown);
        }
        // Room for the grid, which tables being edited do without.
        if !shown {
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
        }
        if ghost_dashes {
            let spans = lined_up.then(|| cells.dash_spans(styling, &lines[1], markup));
            let delimiter_row = &cells.rows[1].cells;
            let dashes = DelimiterDashes::conceal(styling, &lines[1], delimiter_row, spans);
            decorations.delimiter_dashes.push(dashes);
        }
        if !shown {
            decorations
                .grids
                .push(Grid::conceal(styling, lines, &ellipses));
        }
    }
}

/// The cells of a table, as they are shown.
struct Cells {
    /// The header, the delimiter row and the body.
    rows: Vec<Row>,
}

struct Row {
    /// How many pipes there are.
    pipes: usize,
    cells: Vec<Cell>,
}

struct Cell {
    range: Range<usize>,
    /// Whether a pipe closes it, which lines it up with its column.
    closed: bool,
    /// The character spaced out to line it up: its last, but the last dash
    /// in the delimiter row, so that colons stay at the pipes. `None` if
    /// the cell is empty.
    spaced: Option<usize>,
    /// What it hides besides inline markup, sorted and apart: the dashes
    /// but the last of the delimiter row while ghost dashes are drawn
    /// instead, or what is cut off.
    hidden: Vec<Range<usize>>,
    /// How wide it is shown, in Pango units.
    width: i32,
}

impl Cells {
    /// Measures the cells of the table of `lines`, which hide the dashes
    /// of their delimiter row if `ghost_dashes` are drawn instead.
    fn measure(styling: &Styling, lines: &[TableLine], ghost_dashes: bool, markup: Markup) -> Self {
        let text = styling.text;
        let rows = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let cells = line.all_cells(text).into_iter().enumerate();
                let cells = cells.map(|(column, range)| {
                    let mut spaced = text[range.clone()]
                        .char_indices()
                        .last()
                        .map(|(at, _)| range.start + at);
                    let mut hidden = Vec::new();
                    let dashes = (index == 1).then(|| dashes(text, &range)).flatten();
                    if let Some(dashes) = dashes {
                        spaced = Some(dashes.end - 1);
                        if ghost_dashes {
                            hidden.push(dashes.start..dashes.end - 1);
                        }
                    }
                    let mut cell = Cell {
                        range,
                        closed: column < line.cells.len(),
                        spaced,
                        hidden,
                        width: 0,
                    };
                    cell.width = cell.shown_width(styling, &cell.range, markup);
                    cell
                });
                Row {
                    pipes: line.pipes.len(),
                    cells: cells.collect(),
                }
            })
            .collect();
        Self { rows }
    }

    /// The rows, with the delimiter row only if it is `shown`.
    fn rows(&self, shown: bool) -> impl Iterator<Item = &Row> {
        self.rows
            .iter()
            .enumerate()
            .filter(move |(index, _)| shown || *index != 1)
            .map(|(_, row)| row)
    }

    /// How wide the widest cell of each column is, in Pango units, with
    /// the delimiter row only if it is `shown`.
    fn column_widths(&self, shown: bool) -> Vec<i32> {
        let mut column_widths: Vec<i32> = Vec::new();
        for row in self.rows(shown) {
            for (column, cell) in row.cells.iter().enumerate() {
                match column_widths.get_mut(column) {
                    Some(widest) => *widest = (*widest).max(cell.width),
                    None => column_widths.push(cell.width),
                }
            }
        }
        column_widths
    }

    /// How wide the widest row is with the columns lined up, in pixels.
    fn natural_width(&self, styling: &Styling, markup: Markup) -> i32 {
        // Only tables that show their Markdown are in a monospace font.
        let shown = markup.monospace;
        let column_widths = self.column_widths(shown);
        let pipe = pipe_width(styling, markup.monospace);
        let widest = self.rows(shown).map(|row| {
            let cells = row.cells.iter().zip(&column_widths);
            let cells: i32 = cells
                .map(|(cell, widest)| if cell.closed { *widest } else { cell.width })
                .sum();
            cells + pipe * row.pipes as i32
        });
        let widest = widest.max().unwrap_or(0);
        (widest + pango::SCALE - 1) / pango::SCALE
    }

    /// Cuts off the cells too wide for the table to fit in `room` pixels,
    /// the widest first. Returns where ellipses go, see `Cell::cut`.
    fn cut(&mut self, styling: &Styling, room: i32, markup: Markup) -> Vec<(usize, f32)> {
        let pipes = self.rows.iter().map(|row| row.pipes).max().unwrap_or(0) as i32;
        let room = room * pango::SCALE - pipe_width(styling, markup.monospace) * pipes;
        let shortest = styling.width(SHORTEST_CUT) * pango::SCALE;
        let cap = cap(&self.column_widths(false), room).max(shortest);
        let mut ellipses = Vec::new();
        for (index, row) in self.rows.iter_mut().enumerate() {
            if index == 1 {
                continue;
            }
            for cell in &mut row.cells {
                if cell.width > cap
                    && let Some(ellipsis) = cell.cut(styling, cap, markup)
                {
                    ellipses.push(ellipsis);
                }
            }
        }
        ellipses
    }

    /// Where the dashes of each cell of the delimiter `line` go, with the
    /// columns lined up, in Pango units from the start of the line: from
    /// after what is before them in the cell to before what is after. As
    /// the columns end there, exactly where the dashes are once the
    /// cursor is in the row, unlike where GTK says, in whole pixels.
    fn dash_spans(&self, styling: &Styling, line: &TableLine, markup: Markup) -> Vec<(i32, i32)> {
        let column_widths = self.column_widths(true);
        let pipe = pipe_width(styling, markup.monospace);
        let cells = &self.rows[1].cells;
        let width = |range: Range<usize>| text_width(styling, &range, &[], markup);
        let mut column_start = cells
            .first()
            .map_or(0, |cell| width(line.range.start..cell.range.start));
        let mut spans = Vec::new();
        for (cell, widest) in cells.iter().zip(&column_widths) {
            if let Some(dashes) = dashes(styling.text, &cell.range) {
                let cell_width = if cell.closed { *widest } else { cell.width };
                spans.push((
                    column_start + width(cell.range.start..dashes.start),
                    column_start + cell_width - width(dashes.end..cell.range.end),
                ));
            }
            column_start += widest + pipe;
        }
        spans
    }

    /// Lines up the columns, with the delimiter row if it is `shown`: the
    /// spaced character of each cell is spaced out to the width of the
    /// widest cell of its column. Rounded to the spacing step along the
    /// row, so that the rounding does not add up.
    fn line_up(&self, styling: &Styling, shown: bool) {
        let column_widths = self.column_widths(shown);
        for row in self.rows(shown) {
            // How much the row is to be spaced out so far, and how much it is.
            let (mut wanted, mut spaced_out) = (0, 0);
            for (cell, widest) in row.cells.iter().zip(&column_widths) {
                let Some(spaced) = cell.spaced.filter(|_| cell.closed) else {
                    continue;
                };
                wanted += widest - cell.width;
                let steps = ((wanted - spaced_out) as f64 / f64::from(SPACING_STEP)).round();
                let units = (steps as i32).max(0) * SPACING_STEP;
                if units == 0 {
                    continue;
                }
                spaced_out += units;
                let at = styling.offset(spaced);
                let spacing = tags::spacing_units(&styling.buffer, units);
                tags::apply(&styling.buffer, &spacing, at..at + 1);
            }
        }
    }
}

impl Cell {
    /// What of the cell is hidden, as inline markup or its own, sorted
    /// and apart.
    fn all_hidden(&self, markup: Markup) -> Vec<Range<usize>> {
        let mut parts: Vec<Range<usize>> = markup
            .hidden
            .iter()
            .chain(&self.hidden)
            .map(|part| part.start.max(self.range.start)..part.end.min(self.range.end))
            .filter(|part| !part.is_empty())
            .collect();
        parts.sort_by_key(|part| part.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for part in parts {
            match merged.last_mut() {
                Some(last) if part.start <= last.end => last.end = last.end.max(part.end),
                _ => merged.push(part),
            }
        }
        merged
    }

    /// How wide `range` of the cell is shown, in Pango units.
    fn shown_width(&self, styling: &Styling, range: &Range<usize>, markup: Markup) -> i32 {
        text_width(styling, range, &self.all_hidden(markup), markup)
    }

    /// Cuts off the cell to be no wider than `cap`, at its end, or in its
    /// middle if it is a URL, whose end stays. The first characters cut
    /// off are concealed, for an ellipsis over them, the others hidden.
    /// Returns where the ellipsis goes, unless the cell keeps its text: at
    /// a byte offset, and in the middle of URLs that many pixels further.
    fn cut(&mut self, styling: &Styling, cap: i32, markup: Markup) -> Option<(usize, f32)> {
        let text = styling.text;
        let cell = &text[self.range.clone()];
        let content = self.range.start + cell.len() - cell.trim_start().len()
            ..self.range.end - (cell.len() - cell.trim_end().len());
        let hidden = self.all_hidden(markup);
        let width = |range: Range<usize>| self.shown_width(styling, &range, markup);
        // Where the characters shown start.
        let starts: Vec<usize> = text[content.clone()]
            .char_indices()
            .map(|(at, _)| content.start + at)
            .filter(|at| !hidden.iter().any(|part| part.contains(at)))
            .collect();
        let shown: String = starts
            .iter()
            .filter_map(|&at| text[at..].chars().next())
            .collect();
        let is_url = shown.contains("://") && !shown.contains(char::is_whitespace);
        let ellipsis = styling.width(ELLIPSIS) * pango::SCALE;
        let around = width(self.range.start..content.start) + width(content.end..self.range.end);
        let room = cap - around - ellipsis;
        let end_room = if is_url { room / URL_END_SHARE } else { 0 };
        let tail = starts.partition_point(|&at| width(at..content.end) > end_room);
        let tail_start = *starts.get(tail).unwrap_or(&content.end);
        let head_room = room - width(tail_start..content.end);
        let mut head = starts[..tail]
            .partition_point(|&at| width(content.start..at) <= head_room)
            .checked_sub(1)?;
        loop {
            let at = starts[head];
            // As many characters as the ellipsis is wide.
            let concealed = starts[head + 1..tail]
                .iter()
                .copied()
                .find(|&end| width(at..end) >= ellipsis)
                .unwrap_or(tail_start);
            let mut hidden = self.hidden.clone();
            hidden.push(concealed..tail_start);
            let mut cut = Self {
                range: self.range.clone(),
                hidden,
                width: 0,
                ..*self
            };
            cut.width = cut.shown_width(styling, &self.range, markup);
            if cut.width <= cap || head == 0 {
                tags::apply(
                    &styling.buffer,
                    tags::CONCEALED,
                    styling.chars(&(at..concealed)),
                );
                tags::apply(
                    &styling.buffer,
                    tags::HIDDEN,
                    styling.chars(&(concealed..tail_start)),
                );
                let shift = if tail_start < content.end {
                    (width(at..concealed) - ellipsis) as f32 / pango::SCALE as f32 / 2.0
                } else {
                    0.0
                };
                *self = cut;
                return Some((at, shift));
            }
            head -= 1;
        }
    }
}

/// The dashes of a `cell` of a delimiter row in `text`, like `---` in
/// ` :---: `.
fn dashes(text: &str, cell: &Range<usize>) -> Option<Range<usize>> {
    let start = text[cell.clone()].find('-')?;
    let end = text[cell.clone()].rfind('-')? + 1;
    Some(cell.start + start..cell.start + end)
}

/// How wide the cells of columns as wide as `widths` may be for them all
/// to fit in `room`: as wide as they are, but the widest are cut to the
/// same width. `i32::MAX` if they fit as they are.
fn cap(widths: &[i32], room: i32) -> i32 {
    let mut widths = widths.to_vec();
    widths.sort_unstable();
    let mut room = room;
    for (index, width) in widths.iter().enumerate() {
        let share = room / (widths.len() - index) as i32;
        if *width > share {
            return share;
        }
        room -= width;
    }
    i32::MAX
}

/// How wide a pipe is shown, in Pango units, in a `monospace` font or not.
fn pipe_width(styling: &Styling, monospace: bool) -> i32 {
    let layout = styling.layout("|");
    if monospace {
        let attributes = pango::AttrList::new();
        attributes.insert(pango::AttrString::new_family("monospace"));
        layout.set_attributes(Some(&attributes));
    }
    layout.size().0
}

/// How wide `range` of the text is shown, in Pango units, with the styles
/// of `markup` that change widths, in its font, and without the `hidden`
/// markup, which is sorted and apart.
fn text_width(
    styling: &Styling,
    range: &Range<usize>,
    hidden: &[Range<usize>],
    markup: Markup,
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
    if markup.monospace {
        attributes.insert(pango::AttrString::new_family("monospace"));
    }
    // Where `offset` of the text is in what is shown.
    let index = |offset: usize| {
        let hidden_before: usize = hidden
            .iter()
            .filter(|part| part.start < offset)
            .map(|part| part.end.min(offset) - part.start)
            .sum();
        u32::try_from(offset - range.start - hidden_before).unwrap_or(u32::MAX)
    };
    for (style_range, style) in markup.styles {
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

#[cfg(test)]
mod tests {
    use super::cap;

    #[test]
    fn widest_columns_are_cut_alike() {
        assert_eq!(cap(&[10, 50, 20], 100), i32::MAX);
        assert_eq!(cap(&[10, 80, 60], 100), 45);
        assert_eq!(cap(&[60, 80, 70], 90), 30);
        assert_eq!(cap(&[], 0), i32::MAX);
    }
}
