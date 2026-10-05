//! Tables: their lines, cells and the pipes between them.

use std::ops::Range;

/// A line of a table: the header, the delimiter row or a row of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLine {
    /// The byte range of the line, without its line break.
    pub range: Range<usize>,
    /// The byte offsets of the pipes that separate the cells.
    pub pipes: Vec<usize>,
    /// The cells that a pipe closes, as the byte ranges between two pipes,
    /// or between the start of the line and a pipe.
    pub cells: Vec<Range<usize>>,
}

impl TableLine {
    /// The cells, with the last one if no pipe closes it, like ` d` in
    /// `c | d`, of the line in `text`.
    pub fn all_cells(&self, text: &str) -> Vec<Range<usize>> {
        let mut cells = self.cells.clone();
        let rest = self.pipes.last().map_or(self.range.start, |&pipe| pipe + 1);
        if !text[rest..self.range.end].trim().is_empty() {
            cells.push(rest..self.range.end);
        }
        cells
    }
}

/// The table of `lines` in `text` tidied up, to replace its range with:
/// a space between the text of each cell and the pipes around it, and as
/// many dashes in each cell of the delimiter row as make it as long as the
/// longest cell of its column, in characters, but at least three. As in a
/// monospace font, the row then reaches across the table. In other fonts,
/// the editor lines up the columns by spacing out the last character of
/// each cell, which must be a space, or it tears a word apart. `None` if
/// the table is tidy already.
pub fn tidied_table(text: &str, lines: &[TableLine]) -> Option<(Range<usize>, String)> {
    let (first, last) = (lines.first()?, lines.last()?);
    let padded: Vec<(String, Vec<usize>)> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 1 {
                (String::new(), Vec::new())
            } else {
                padded_line(text, line)
            }
        })
        .collect();
    let mut tidied = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            tidied.push_str(&text[lines[index - 1].range.end..line.range.start]);
        }
        if index == 1 {
            let longest = |column: usize| {
                padded
                    .iter()
                    .filter_map(|(_, lengths)| lengths.get(column).copied())
                    .max()
            };
            tidied.push_str(&filled_delimiter_row(text, line, longest)?);
        } else {
            tidied.push_str(&padded[index].0);
        }
    }
    let range = first.range.start..last.range.end;
    (tidied != text[range.clone()]).then_some((range, tidied))
}

/// The table `line` of `text` with a space between the text of each cell
/// and the pipes around it, and the length of each cell then, in
/// characters.
fn padded_line(text: &str, line: &TableLine) -> (String, Vec<usize>) {
    let mut padded = String::new();
    let mut lengths = Vec::new();
    let mut start = line.range.start;
    for (column, cell) in line.all_cells(text).into_iter().enumerate() {
        padded.push_str(&text[start..cell.start]);
        let content = &text[cell.clone()];
        let is_empty = content.trim().is_empty();
        let opened = line.pipes.contains(&cell.start.wrapping_sub(1));
        let closed = column < line.cells.len();
        let space = |pipe: bool, spaced: bool| {
            if pipe && !is_empty && !spaced {
                " "
            } else {
                ""
            }
        };
        let before = space(opened, content.starts_with(char::is_whitespace));
        let after = space(closed, content.ends_with(char::is_whitespace));
        let padded_cell = format!("{before}{content}{after}");
        lengths.push(padded_cell.chars().count());
        padded.push_str(&padded_cell);
        start = cell.end;
    }
    padded.push_str(&text[start..line.range.end]);
    (padded, lengths)
}

/// The delimiter row `line` of `text` with as many dashes in each cell as
/// make it as `longest` as the cells of its column, but at least three.
/// `None` if a cell has no dashes.
fn filled_delimiter_row(
    text: &str,
    line: &TableLine,
    longest: impl Fn(usize) -> Option<usize>,
) -> Option<String> {
    let mut filled = String::new();
    let mut start = line.range.start;
    for (column, cell) in line.all_cells(text).iter().enumerate() {
        let Some(longest) = longest(column) else {
            continue;
        };
        let dashes = text[cell.clone()].trim().trim_matches(':');
        let around = text[cell.clone()].chars().count() - dashes.len();
        let dashes_start = cell.start + text[cell.clone()].find('-')?;
        filled.push_str(&text[start..dashes_start]);
        filled.push_str(&"-".repeat(longest.saturating_sub(around).max(3)));
        start = dashes_start + dashes.len();
    }
    filled.push_str(&text[start..line.range.end]);
    Some(filled)
}

/// The lines of the table at `range` of `text`: first the header, then
/// the delimiter row, then the body.
pub(super) fn table_lines(text: &str, range: &Range<usize>) -> Vec<TableLine> {
    let mut start = range.start;
    text[range.clone()]
        .split_inclusive('\n')
        .map(|line| {
            let table_line = table_line(line.trim_end_matches(['\n', '\r']), start);
            start += line.len();
            table_line
        })
        .collect()
}

/// The table `line` at `start`, without its line break.
fn table_line(line: &str, start: usize) -> TableLine {
    let mut pipes = Vec::new();
    let mut cells = Vec::new();
    let mut cell_start = 0;
    let mut escaped = false;
    for (index, c) in line.char_indices() {
        if c == '|' && !escaped {
            // A pipe that starts the line opens the first cell.
            if !pipes.is_empty() || !line[..index].trim().is_empty() {
                cells.push(start + cell_start..start + index);
            }
            pipes.push(start + index);
            cell_start = index + 1;
        }
        escaped = c == '\\' && !escaped;
    }
    TableLine {
        range: start..start + line.len(),
        pipes,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::tidied_table;
    use crate::markdown::{MarkdownMode, MarkdownStyle, markdown_formatting};

    /// The first table in `text` once tidied.
    fn tidied(text: &str) -> Option<String> {
        let tables = markdown_formatting(text, MarkdownMode::Full).tables;
        tidied_table(text, &tables[0]).map(|(range, table)| {
            let mut text = text.to_owned();
            text.replace_range(range, &table);
            text
        })
    }

    #[test]
    fn delimiter_row_reaches_across() {
        let text = "| Name | Status |\n|---|:-:|\n| Payment provider | open |\n";
        assert_eq!(
            tidied(text).as_deref(),
            Some("| Name | Status |\n|------------------|:------:|\n| Payment provider | open |\n")
        );
        // Spaces and colons stay, too many dashes go.
        let text = "| a | b |\n| :--------- | ---: |\n| ä | c |\n";
        assert_eq!(
            tidied(text).as_deref(),
            Some("| a | b |\n| :--- | ---: |\n| ä | c |\n")
        );
        // Without outer pipes, and with a last cell no pipe closes.
        let text = "Name | Status\n--- | ---\nPayment provider | open\n";
        assert_eq!(
            tidied(text).as_deref(),
            Some("Name | Status\n---------------- | ------\nPayment provider | open\n")
        );
        assert_eq!(tidied("| Name |\n|------|\n| x |\n"), None);
    }

    #[test]
    fn cells_get_spaces_at_their_pipes() {
        let text = "|Name|Link|\n|:-|-:|\n|ass|asd|\n|| x|\n\nText\n";
        assert_eq!(
            tidied(text).as_deref(),
            Some("| Name | Link |\n|:-----|-----:|\n| ass | asd |\n|| x |\n\nText\n")
        );
        // Indented, without outer pipes, with an escaped pipe.
        let text = "- a\n\n  b|c\\|\n  -|-\n";
        assert_eq!(
            tidied(text).as_deref(),
            Some("- a\n\n  b | c\\|\n  ---|----\n")
        );
    }

    #[test]
    fn table_header_and_cells() {
        let text = "Intro\n\n| Name | Wert |\n|---|:-:|\n| a \\| b | `1` |\nc | d\n";
        let formatting = markdown_formatting(text, MarkdownMode::Block);
        let header = formatting
            .styles
            .iter()
            .find(|(_, style)| *style == MarkdownStyle::TableHeader)
            .map(|(range, _)| &text[range.clone()]);
        assert_eq!(header, Some("| Name | Wert |\n"));
        let tables = formatting.tables;
        let cells: Vec<Vec<&str>> = tables[0]
            .iter()
            .map(|line| line.cells.iter().map(|cell| &text[cell.clone()]).collect())
            .collect();
        assert_eq!(
            cells,
            [
                vec![" Name ", " Wert "],
                vec!["---", ":-:"],
                vec![" a \\| b ", " `1` "],
                vec!["c "],
            ]
        );
        let lines: Vec<&str> = tables[0]
            .iter()
            .map(|line| &text[line.range.clone()])
            .collect();
        assert_eq!(lines[3], "c | d");
        let pipes: Vec<usize> = tables[0][3]
            .pipes
            .iter()
            .map(|&pipe| pipe - tables[0][3].range.start)
            .collect();
        assert_eq!(pipes, [2]);
        // The escaped pipe separates no cells.
        assert_eq!(tables[0][2].pipes.len(), 3);
    }
}
