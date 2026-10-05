//! Lists: their items, and how their lines are indented.

use std::ops::Range;

use super::{Formatting, MarkdownStyle, line_start, lines, next_line_start};

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

/// Makes the lines with nothing but a bullet and a space, like `  - `, the
/// empty list items they are about to become while being typed. Below the
/// text of an item or paragraph, CommonMark makes them none: an empty item
/// cannot interrupt a paragraph, so the bullet goes on with its text, or
/// `-` makes the text a setext heading.
pub(super) fn add_typed_items(text: &str, formatting: &mut Formatting) {
    let typed: Vec<ListItem> = lines(text)
        .filter_map(|line| typed_item(text, line, formatting))
        .collect();
    for item in typed {
        let marker = item.range.start;
        formatting.styles.retain(|(range, style)| match style {
            MarkdownStyle::Heading(_) => !range.contains(&marker),
            MarkdownStyle::Markup => range.start != marker,
            _ => true,
        });
        formatting
            .inline_markup
            .retain(|markup| markup.element.end != marker + 1);
        formatting
            .styles
            .push((marker..marker + 1, MarkdownStyle::Bullet(item.depth)));
        formatting.list_items.push(item);
    }
}

/// The empty list item that `line` is about to become, if it has nothing
/// but a bullet and a space and is no list item yet. Deeper than the item
/// it is in if it is indented as far as the text of that item.
fn typed_item(text: &str, line: Range<usize>, formatting: &Formatting) -> Option<ListItem> {
    let marker = line.start + text[line.clone()].find(|c: char| !c.is_whitespace())?;
    if !text[marker..].starts_with(['-', '*', '+']) {
        return None;
    }
    let rest = &text[marker + 1..line.end];
    let is_typed = !rest.is_empty()
        && rest.trim_matches([' ', '\t']).is_empty()
        && !formatting
            .list_items
            .iter()
            .any(|item| item.range.start == marker)
        && !formatting
            .code_blocks
            .iter()
            .any(|block| block.range.contains(&marker));
    if !is_typed {
        return None;
    }
    let depth = match innermost_item(&formatting.list_items, marker) {
        Some(item) if marker - line.start >= item.content - item.line_start => {
            item.depth.saturating_add(1)
        }
        Some(item) => item.depth,
        None => 1,
    };
    Some(list_item(text, marker..line.end, depth))
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
    use crate::markdown::{MarkdownMode, MarkdownStyle, markdown_formatting};

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

    #[test]
    fn bullets_show_once_a_space_follows() {
        let bullets = |text: &'static str| -> Vec<(&str, u8)> {
            markdown_formatting(text, MarkdownMode::Block)
                .styles
                .into_iter()
                .filter_map(|(range, style)| match style {
                    MarkdownStyle::Bullet(depth) => Some((&text[range], depth)),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(bullets("- a\n-"), [("-", 1)]);
        assert_eq!(bullets("- a\n- "), [("-", 1), ("-", 1)]);
        // Empty items below text, which CommonMark has none for yet.
        assert_eq!(bullets("- a\n  - "), [("-", 1), ("-", 2)]);
        assert_eq!(bullets("- a\n  * "), [("-", 1), ("*", 2)]);
        assert_eq!(bullets("Text\n- "), [("-", 1)]);
        assert!(bullets("Text\n  -").is_empty());
        assert!(bullets("```\nText\n- \n```").is_empty());
        let indents = markdown_formatting("- a\n  - ", MarkdownMode::Block).list_indents;
        assert_eq!(indents[0].depth, 2);
        // Not a setext heading either.
        let formatting = markdown_formatting("Text\n- ", MarkdownMode::Full);
        assert_eq!(formatting.styles, [(5..6, MarkdownStyle::Bullet(1))]);
        assert!(formatting.inline_markup.is_empty());
    }
}
