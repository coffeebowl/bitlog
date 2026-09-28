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
    use crate::markdown::{MarkdownMode, MarkdownStyle, markdown_formatting};

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
