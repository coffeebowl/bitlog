//! Lists: their items, and how their lines are indented.

use std::ops::Range;

use super::{line_start, lines, next_line_start};

/// The indent of a line of a list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListIndent {
    /// The spaces at the start of the line.
    pub spaces: Range<usize>,
    /// How deep the list of the innermost item of the line is, from 1.
    pub depth: u8,
    /// On lines after the first of an item, its marker and the spaces
    /// after it, like `1. `, whose text the line lines up with.
    pub marker: Option<Range<usize>>,
}

/// An item of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ListItem {
    /// From its marker, like `-` or `1.`, to its end, with nested items.
    pub(super) range: Range<usize>,
    /// How deep its list is nested, from 1.
    pub(super) depth: u8,
    /// Where the line of its marker starts.
    pub(super) line_start: usize,
    /// Where its content starts, after the marker and its spaces.
    pub(super) content: usize,
}

/// The spaces at the start of each line of `text` in one of `items`.
pub(super) fn list_indents(text: &str, items: &[ListItem]) -> Vec<ListIndent> {
    lines(text)
        .filter_map(|line| {
            let first = line.start + text[line.clone()].find(|c: char| !c.is_whitespace())?;
            let item = innermost_item(items, first)?;
            let marker = (item.range.start != first).then_some(item.range.start..item.content);
            Some(ListIndent {
                spaces: line.start..first,
                depth: item.depth,
                marker,
            })
        })
        .filter(|indent| !indent.spaces.is_empty() || indent.marker.is_some())
        .collect()
}

/// The innermost of `items` with the text at `at`.
pub(super) fn innermost_item(items: &[ListItem], at: usize) -> Option<&ListItem> {
    items
        .iter()
        .filter(|item| item.range.contains(&at))
        .max_by_key(|item| item.depth)
}

pub(super) fn list_item(text: &str, range: Range<usize>, depth: u8) -> ListItem {
    let line = &text[range.start..next_line_start(text, range.start)];
    let marker = line.find(char::is_whitespace).unwrap_or(line.len());
    let spaces = line[marker..].len() - line[marker..].trim_start_matches([' ', '\t']).len();
    ListItem {
        line_start: line_start(text, range.start),
        content: range.start + marker + spaces,
        range,
        depth,
    }
}

#[cfg(test)]
mod tests {
    use crate::markdown::{MarkdownMode, markdown_formatting};

    #[test]
    fn list_indents() {
        let text = "- a\n  more\n  1. b\n\n     c\n";
        let indents: Vec<(&str, u8, Option<&str>)> = markdown_formatting(text, MarkdownMode::Block)
            .list_indents
            .into_iter()
            .map(|indent| {
                (
                    &text[indent.spaces],
                    indent.depth,
                    indent.marker.map(|marker| &text[marker]),
                )
            })
            .collect();
        assert_eq!(
            indents,
            [
                ("  ", 1, Some("- ")),
                ("  ", 2, None),
                ("     ", 2, Some("1. ")),
            ]
        );
    }
}
