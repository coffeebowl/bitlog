//! Where Markdown text is formatted, for live formatting in the app.

use std::cmp::Reverse;
use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

/// Which Markdown a text may use, see "Block Markdown" in the format spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownMode {
    /// Block texts and day notes: headings are plain text.
    Block,
    /// Project notes: everything, headings included.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownStyle {
    Strong,
    Emphasis,
    Strikethrough,
    Code,
    CodeBlock,
    /// The text of a link, not its target.
    Link,
    Quote,
    /// Level 1 to 6, only in [`MarkdownMode::Full`].
    Heading(u8),
    /// Markdown syntax like `**`, `> ` or `](target)`, shown dimmed.
    Markup,
}

/// An element being parsed, with the ranges of its direct children.
struct Frame<'a> {
    tag: Tag<'a>,
    range: Range<usize>,
    children: Vec<Range<usize>>,
}

/// The formatted byte ranges of `text`, sorted by start, outer ranges first.
/// Ranges may nest.
pub fn markdown_styles(text: &str, mode: MarkdownMode) -> Vec<(Range<usize>, MarkdownStyle)> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut styles = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                stack.push(Frame {
                    tag,
                    range,
                    children: Vec::new(),
                });
                continue;
            }
            Event::End(_) => {
                let frame = stack.pop().expect("every end has a start");
                style_element(text, &frame, mode, &mut styles);
            }
            Event::Code(_) => {
                styles.push((range.clone(), MarkdownStyle::Code));
                let ticks = |part: &str| part.len() - part.trim_matches('`').len();
                let code = &text[range.clone()];
                let open = code.len() - code.trim_start_matches('`').len();
                let close = ticks(code) - open;
                styles.push((range.start..range.start + open, MarkdownStyle::Markup));
                styles.push((range.end - close..range.end, MarkdownStyle::Markup));
            }
            Event::Rule | Event::TaskListMarker(_) => {
                if let Some(markup) = trim(text, range.clone()) {
                    styles.push((markup, MarkdownStyle::Markup));
                }
            }
            _ => {}
        }
        if let Some(parent) = stack.last_mut() {
            parent.children.push(range);
        }
    }
    // Outer ranges first.
    styles.sort_by_key(|(range, _)| (range.start, Reverse(range.end)));
    styles
}

fn style_element(
    text: &str,
    frame: &Frame,
    mode: MarkdownMode,
    styles: &mut Vec<(Range<usize>, MarkdownStyle)>,
) {
    let range = frame.range.clone();
    let style = match &frame.tag {
        Tag::Strong => Some(MarkdownStyle::Strong),
        Tag::Emphasis => Some(MarkdownStyle::Emphasis),
        Tag::Strikethrough => Some(MarkdownStyle::Strikethrough),
        Tag::CodeBlock(_) => Some(MarkdownStyle::CodeBlock),
        Tag::BlockQuote(_) => Some(MarkdownStyle::Quote),
        Tag::Heading { .. } if mode == MarkdownMode::Block => return,
        Tag::Heading { level, .. } => Some(MarkdownStyle::Heading(*level as u8)),
        Tag::Link { .. } => {
            for child in &frame.children {
                styles.push((child.clone(), MarkdownStyle::Link));
            }
            None
        }
        _ => None,
    };
    if let Some(style) = style {
        styles.push((range.clone(), style));
    }
    for gap in gaps(range, &frame.children) {
        if let Some(markup) = trim(text, gap) {
            styles.push((markup, MarkdownStyle::Markup));
        }
    }
}

/// The parts of `range` that none of `children` covers.
fn gaps(range: Range<usize>, children: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut gaps = Vec::new();
    let mut cursor = range.start;
    for child in children {
        if child.start > cursor {
            gaps.push(cursor..child.start);
        }
        cursor = cursor.max(child.end);
    }
    if cursor < range.end {
        gaps.push(cursor..range.end);
    }
    gaps
}

/// `range` without surrounding whitespace, or `None` if nothing else is left.
fn trim(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let part = &text[range.clone()];
    let start = range.start + (part.len() - part.trim_start().len());
    let end = range.end - (part.len() - part.trim_end().len());
    (start < end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use MarkdownStyle::*;

    /// The styled parts of `text`, as text for readable assertions.
    fn styled(text: &str, mode: MarkdownMode) -> Vec<(&str, MarkdownStyle)> {
        markdown_styles(text, mode)
            .into_iter()
            .map(|(range, style)| (&text[range], style))
            .collect()
    }

    #[test]
    fn inline_styles_and_their_markup() {
        assert_eq!(
            styled("A **bold** and *it* ~~no~~", MarkdownMode::Block),
            [
                ("**bold**", Strong),
                ("**", Markup),
                ("**", Markup),
                ("*it*", Emphasis),
                ("*", Markup),
                ("*", Markup),
                ("~~no~~", Strikethrough),
                ("~~", Markup),
                ("~~", Markup),
            ]
        );
    }

    #[test]
    fn nested_styles() {
        assert_eq!(
            styled("***both***", MarkdownMode::Block),
            [
                ("***both***", Emphasis),
                ("*", Markup),
                ("**both**", Strong),
                ("**", Markup),
                ("**", Markup),
                ("*", Markup),
            ]
        );
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            styled("Run ``a `b` c`` now", MarkdownMode::Block),
            [("``a `b` c``", Code), ("``", Markup), ("``", Markup)]
        );
    }

    #[test]
    fn links_style_their_text() {
        assert_eq!(
            styled("See [the docs](https://example.com).", MarkdownMode::Block),
            [
                ("[", Markup),
                ("the docs", Link),
                ("](https://example.com)", Markup),
            ]
        );
    }

    #[test]
    fn lists_quotes_and_code_blocks() {
        let text = "- one\n- [x] two\n\n> quoted\n> more\n\n```rust\nlet a = 1;\n```\n";
        assert_eq!(
            styled(text, MarkdownMode::Block),
            [
                ("-", Markup),
                ("-", Markup),
                ("[x]", Markup),
                ("> quoted\n> more\n", Quote),
                (">", Markup),
                (">", Markup),
                ("```rust\nlet a = 1;\n```", CodeBlock),
                ("```rust", Markup),
                ("```", Markup),
            ]
        );
    }

    #[test]
    fn headings_depend_on_the_mode() {
        let text = "# Title\n\nText\n===\n";
        assert_eq!(styled(text, MarkdownMode::Block), []);
        assert_eq!(
            styled(text, MarkdownMode::Full),
            [
                ("# Title\n", Heading(1)),
                ("#", Markup),
                ("Text\n===\n", Heading(1)),
                ("===", Markup),
            ]
        );
    }

    #[test]
    fn escaped_heading_dims_the_backslash() {
        assert_eq!(
            styled("\\# Not a heading", MarkdownMode::Block),
            [("\\", Markup)]
        );
    }

    #[test]
    fn rules_are_markup() {
        assert_eq!(
            styled("a\n\n***\n\nb", MarkdownMode::Block),
            [("***", Markup)]
        );
    }

    #[test]
    fn multibyte_text() {
        assert_eq!(
            styled("Grüße **schön**", MarkdownMode::Block),
            [("**schön**", Strong), ("**", Markup), ("**", Markup)]
        );
    }
}
