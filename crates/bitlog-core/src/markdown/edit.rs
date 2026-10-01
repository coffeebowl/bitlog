//! What Enter and Tab do in lists and quotes, as edits of the text.

use std::ops::Range;

use super::lists::innermost_item;
use super::{MarkdownMode, MarkdownStyle, line_start, lines, markdown_formatting, next_line_start};

/// What Enter does in a list item or quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// Starts the next item or line of the quote: inserts this at the
    /// cursor.
    Insert(String),
    /// Ends the list or quote, as the item or line is empty: removes this
    /// range, its marker.
    End(Range<usize>),
    /// Nests the empty item less deep.
    Outdent,
}

/// What Enter does with the cursor at `at` in `text`, if it is after the
/// marker of a list item: start the next item, with the same marker, the
/// next number or an empty check box, or end or outdent the list if the
/// item is empty. `None` if it is in no list item.
pub fn continue_list(text: &str, mode: MarkdownMode, at: usize) -> Option<Continuation> {
    let items = markdown_formatting(text, mode).list_items;
    let line = line_start(text, at);
    let first = line + text[line..].find(|c: char| !c.is_whitespace())?;
    let item = innermost_item(&items, first)?;
    // Enter before the text of the item only breaks the line.
    if at < item.content && item.line_start == line {
        return None;
    }
    let line_end = next_line_start(text, at);
    let rest = text[item.content.min(line_end)..line_end].trim();
    let task = ["[ ]", "[x]", "[X]"]
        .into_iter()
        .find(|task| rest.starts_with(task));
    let is_empty = item.line_start == line
        && task
            .map_or(rest, |task| rest[task.len()..].trim())
            .is_empty()
        && text[at..line_end].trim().is_empty();
    if is_empty {
        return Some(if item.depth > 1 {
            Continuation::Outdent
        } else {
            Continuation::End(item.range.start..line_end.min(text.len()).max(at))
        });
    }
    let marker = text[item.range.start..item.content].trim_end();
    let digits = marker.len()
        - marker
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    let next_marker = match marker[..digits].parse::<u64>() {
        Ok(number) => format!("{}{}", number.saturating_add(1), &marker[digits..]),
        Err(_) => marker.to_owned(),
    };
    let indent = &text[item.line_start..item.range.start];
    let task = if task.is_some() { "[ ] " } else { "" };
    Some(Continuation::Insert(format!(
        "\n{indent}{next_marker} {task}"
    )))
}

/// What Enter does with the cursor at `at` in `text`, if it is after the
/// `>` of a line in a quote: start the next line of the quote, or end it if
/// the line is empty. `None` if it is in no quote.
pub fn continue_quote(text: &str, mode: MarkdownMode, at: usize) -> Option<Continuation> {
    let in_quote = markdown_formatting(text, mode)
        .styles
        .iter()
        .any(|(range, style)| {
            *style == MarkdownStyle::Quote && range.start <= at && at <= range.end
        });
    let line = line_start(text, at);
    let line_end = text[line..].find('\n').map_or(text.len(), |end| line + end);
    let content = &text[line..line_end];
    let prefix = content.len() - content.trim_start_matches(['>', ' ', '\t']).len();
    // The cursor may be before the space after the last `>`.
    let markers = content[..prefix].trim_end().len();
    if !in_quote || markers == 0 || at < line + markers {
        return None;
    }
    if content[prefix..].trim().is_empty() {
        return Some(Continuation::End(line..line_end));
    }
    Some(Continuation::Insert(format!("\n{}", &content[..prefix])))
}

/// How to nest the list item on the line of `at` in `text` one level
/// `deeper` or less deep, with all its lines: the byte ranges to replace
/// and what with, in order. It goes below the item before it, or beside
/// the item it is in. Empty if it cannot, `None` if `at` is in no list.
pub fn nest_list_item(
    text: &str,
    mode: MarkdownMode,
    at: usize,
    deeper: bool,
) -> Option<Vec<(Range<usize>, String)>> {
    let items = markdown_formatting(text, mode).list_items;
    let line = line_start(text, at);
    let first = line + text[line..].find(|c: char| !c.is_whitespace())?;
    let item = innermost_item(&items, first)?;
    let indent = &text[item.line_start..item.range.start];
    // Items in quotes are nested by more than spaces.
    if indent.chars().any(|c| c != ' ') {
        return Some(Vec::new());
    }
    let column = indent.len();
    let target = if deeper {
        items
            .iter()
            .filter(|before| {
                before.depth == item.depth
                    && before.range.end <= item.range.start
                    && text[before.range.end..item.range.start].trim().is_empty()
            })
            .map(|before| before.content - before.line_start)
            .next()
    } else {
        items
            .iter()
            .filter(|outer| {
                outer.depth + 1 == item.depth && outer.range.contains(&item.range.start)
            })
            .map(|outer| outer.range.start - outer.line_start)
            .next()
    };
    let Some(target) = target.filter(|&target| target != column) else {
        return Some(Vec::new());
    };
    let mut edits: Vec<(Range<usize>, String)> = lines(text)
        .filter(|line| {
            (item.line_start..item.range.end).contains(&line.start)
                && !text[line.clone()].trim().is_empty()
        })
        .map(|line| {
            if deeper {
                (line.start..line.start, " ".repeat(target - column))
            } else {
                let spaces =
                    text[line.clone()].len() - text[line.clone()].trim_start_matches(' ').len();
                (
                    line.start..line.start + spaces.min(column - target),
                    String::new(),
                )
            }
        })
        .collect();
    // A numbered item that goes deeper starts a list of its own, or is no
    // more than one of the items after the first, whose numbers count not.
    let digits = text[item.range.start..].len()
        - text[item.range.start..]
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    if deeper && digits > 0 {
        edits.insert(
            1,
            (item.range.start..item.range.start + digits, "1".to_owned()),
        );
    }
    Some(edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Enter does at the end of the line with `at` in `text`.
    fn enter(text: &str, at: &str) -> Option<Continuation> {
        let line = text.find(at).expect("the text has it");
        let end = text[line..].find('\n').map_or(text.len(), |end| line + end);
        continue_list(text, MarkdownMode::Block, end)
    }

    #[test]
    fn enter_continues_quotes() {
        use Continuation::*;
        let enter = |text: &str| continue_quote(text, MarkdownMode::Block, text.trim_end().len());
        assert_eq!(enter("> a"), Some(Insert("\n> ".to_owned())));
        assert_eq!(enter("> > a\n"), Some(Insert("\n> > ".to_owned())));
        assert_eq!(enter("> [!NOTE]\n> a"), Some(Insert("\n> ".to_owned())));
        assert_eq!(enter("> a\n> "), Some(End(4..6)));
        assert_eq!(enter("a > b"), None);
        assert_eq!(enter("```\n> code\n```"), None);
    }

    #[test]
    fn enter_continues_lists() {
        use Continuation::*;
        let insert = |text: &str| Some(Insert(text.to_owned()));
        assert_eq!(enter("- a\n", "a"), insert("\n- "));
        assert_eq!(enter("* [x] a\n", "a"), insert("\n* [ ] "));
        assert_eq!(enter("1. a\n2. b\n", "b"), insert("\n3. "));
        assert_eq!(enter("- a\n  - b\n", "b"), insert("\n  - "));
        assert_eq!(enter("9) a", "a"), insert("\n10) "));
        // Empty items end the list, or go up a level.
        assert_eq!(enter("- a\n- \n", "- \n"), Some(End(4..7)));
        assert_eq!(enter("- a\n- [ ] \n", "- [ ]"), Some(End(4..11)));
        assert_eq!(enter("- a\n  - b\n  - \n", "  - \n"), Some(Outdent));
        assert_eq!(enter("Text\n", "Text"), None);
    }

    /// `text` with the list item at `at` nested `deeper` or less deep.
    fn nested(text: &str, at: &str, deeper: bool) -> Option<String> {
        let at = text.find(at).expect("the text has it");
        let edits = nest_list_item(text, MarkdownMode::Block, at, deeper)?;
        let mut nested = text.to_owned();
        for (range, replacement) in edits.into_iter().rev() {
            nested.replace_range(range, &replacement);
        }
        Some(nested)
    }

    #[test]
    fn nest_list_items() {
        let text = "- a\n- b\n  more\n  - c\n- d\n";
        // Below `a`, with what belongs to it.
        assert_eq!(
            nested(text, "b", true).as_deref(),
            Some("- a\n  - b\n    more\n    - c\n- d\n")
        );
        assert_eq!(
            nested(text, "c", false).as_deref(),
            Some("- a\n- b\n  more\n- c\n- d\n")
        );
        // Ordered items nest by the width of their marker, and start a
        // list of their own.
        assert_eq!(
            nested("1. a\n2. b\n", "b", true).as_deref(),
            Some("1. a\n   1. b\n")
        );
        assert_eq!(
            nested("1. a\n   1. b\n", "b", false).as_deref(),
            Some("1. a\n1. b\n")
        );
        // The first item has none to go below, top items none to leave.
        assert_eq!(nested(text, "a", true).as_deref(), Some(text));
        assert_eq!(nested(text, "d", false).as_deref(), Some(text));
        assert_eq!(nested("Text\n", "Text", true), None);
    }
}
