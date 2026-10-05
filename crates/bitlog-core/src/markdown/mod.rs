//! Where Markdown text is formatted, for live formatting in the app.

mod code_blocks;
mod edit;
mod headings;
mod lists;
mod tables;

pub use code_blocks::CodeBlock;
pub use edit::{Continuation, continue_list, continue_quote, nest_list_item};
pub use headings::{escape_headings, heading_lines};
pub use lists::ListIndent;
pub use tables::TableLine;

use std::cmp::Reverse;
use std::ops::Range;

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, LinkType, Options, Parser, Tag};

use self::code_blocks::code_block;
use self::lists::{ListItem, add_typed_items, list_indents, list_item};
use self::tables::table_lines;

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
    /// The `>` of a quote, shown as a bar beside it instead.
    QuoteMarker,
    /// The `-`, `*` or `+` of an item in a bullet list, shown as a bullet
    /// instead, with how deep the list is nested, from 1.
    Bullet(u8),
    /// The number of an item in an ordered list, like `1.`, shown in bold.
    Number,
    /// The first row of a table.
    TableHeader,
    /// A thematic break like `---`, shown as a line instead.
    Rule,
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

/// How a text is formatted, in byte ranges.
#[derive(Debug, Default)]
pub struct Formatting {
    /// Sorted by start, outer ranges first. Ranges may nest.
    pub styles: Vec<(Range<usize>, MarkdownStyle)>,
    /// Inline elements, whose markup is hidden unless the cursor is at
    /// them.
    pub inline_markup: Vec<InlineMarkup>,
    /// The lines of list items that are indented.
    pub list_indents: Vec<ListIndent>,
    /// The markers of list items with the spaces after them, like `1. `.
    pub list_markers: Vec<Range<usize>>,
    pub code_blocks: Vec<CodeBlock>,
    pub tasks: Vec<TaskItem>,
    /// Links to web pages and mail addresses, which open in other apps.
    pub web_links: Vec<WebLink>,
    pub callouts: Vec<Callout>,
    /// The lines of each table: the header, the delimiter row, the body.
    pub tables: Vec<Vec<TableLine>>,
    list_items: Vec<ListItem>,
}

/// An inline element like `**bold**`, or the heading `# Title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineMarkup {
    /// Without the line break of a heading.
    pub element: Range<usize>,
    /// Like the two `**`, or `# ` with its space.
    pub markup: Vec<Range<usize>>,
}

/// A link that another app opens, like a web browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebLink {
    /// All of it, like `[text](https://example.com)`.
    pub range: Range<usize>,
    /// Like `https://example.com`, or `mailto:anna@example.com` for a
    /// mail address.
    pub url: String,
}

/// What a callout is about, as GitHub has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// A quote that starts with its kind, like `> [!NOTE]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callout {
    pub range: Range<usize>,
    pub kind: CalloutKind,
    /// Like `[!NOTE]`.
    pub marker: Range<usize>,
}

/// An item of a task list, like `- [ ] Call Anna`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    /// Of its list item, like `-` or `1.`.
    pub marker: Range<usize>,
    /// Like `[ ]` or `[x]`.
    pub check_box: Range<usize>,
    pub done: bool,
    /// The rest of its line.
    pub text: Range<usize>,
}

pub fn markdown_formatting(text: &str, mode: MarkdownMode) -> Formatting {
    let mut formatting = Formatting::default();
    let mut stack: Vec<Frame> = Vec::new();
    for (event, range) in Parser::new_ext(text, options(mode)).into_offset_iter() {
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
                end_element(text, &frame, &stack, mode, &mut formatting);
            }
            Event::Code(_) => {
                formatting.styles.push((range.clone(), MarkdownStyle::Code));
                let ticks = |part: &str| part.len() - part.trim_matches('`').len();
                let code = &text[range.clone()];
                let open = code.len() - code.trim_start_matches('`').len();
                let close = ticks(code) - open;
                let markup = vec![
                    range.start..range.start + open,
                    range.end - close..range.end,
                ];
                for markup in &markup {
                    formatting
                        .styles
                        .push((markup.clone(), MarkdownStyle::Markup));
                }
                formatting.inline_markup.push(InlineMarkup {
                    element: range.clone(),
                    markup,
                });
            }
            Event::TaskListMarker(done) => {
                let item = stack
                    .iter()
                    .rev()
                    .find(|frame| matches!(frame.tag, Tag::Item));
                if let Some(item) = item {
                    let marker = &text[item.range.start..range.start];
                    let marker_end = marker.find(char::is_whitespace).unwrap_or(marker.len());
                    formatting.tasks.push(TaskItem {
                        marker: item.range.start..item.range.start + marker_end,
                        check_box: range.clone(),
                        done,
                        text: trim(text, range.end..next_line_start(text, range.end))
                            .unwrap_or(range.end..range.end),
                    });
                }
                formatting
                    .styles
                    .push((range.clone(), MarkdownStyle::Markup));
            }
            Event::Rule => {
                if let Some(markup) = trim(text, range.clone()) {
                    formatting
                        .styles
                        .push((markup.clone(), MarkdownStyle::Rule));
                    formatting.styles.push((markup, MarkdownStyle::Markup));
                }
            }
            _ => {}
        }
        if let Some(parent) = stack.last_mut() {
            parent.children.push(range);
        }
    }
    add_typed_items(text, &mut formatting);
    // Outer ranges first.
    formatting
        .styles
        .sort_by_key(|(range, _)| (range.start, Reverse(range.end)));
    formatting.list_indents = list_indents(text, &formatting.list_items);
    formatting.list_markers = formatting
        .list_items
        .iter()
        .map(|item| item.range.start..item.content)
        .collect();
    formatting
}

/// Formats the element of `frame`, inside the elements of `stack`, once it
/// has ended and its children are known.
fn end_element(
    text: &str,
    frame: &Frame,
    stack: &[Frame],
    mode: MarkdownMode,
    formatting: &mut Formatting,
) {
    let in_quote = matches!(frame.tag, Tag::BlockQuote(_))
        || stack
            .iter()
            .any(|frame| matches!(frame.tag, Tag::BlockQuote(_)));
    let inline = style_element(text, frame, mode, in_quote, &mut formatting.styles);
    formatting.inline_markup.extend(inline);
    if let Some(marker) = list_marker(text, frame, stack) {
        formatting.styles.push(marker);
    }
    match &frame.tag {
        Tag::Item => {
            let item = list_item(text, frame.range.clone(), depth(stack));
            formatting.list_items.push(item);
        }
        Tag::CodeBlock(kind) => {
            let info = match kind {
                CodeBlockKind::Fenced(info) => Some(info.as_ref()),
                CodeBlockKind::Indented => None,
            };
            formatting.code_blocks.push(code_block(text, frame, info));
        }
        Tag::Table(_) => formatting.tables.push(table_lines(text, &frame.range)),
        Tag::Link {
            link_type,
            dest_url,
            ..
        } => {
            let url = web_url(*link_type, dest_url);
            if let (Some(url), Some(range)) = (url, trim(text, frame.range.clone())) {
                formatting.web_links.push(WebLink { range, url });
            }
        }
        Tag::BlockQuote(Some(kind)) => {
            let line_end = next_line_start(text, frame.range.start);
            let marker = text[frame.range.start..line_end]
                .find("[!")
                .zip(text[frame.range.start..line_end].find(']'))
                .map(|(start, end)| frame.range.start + start..frame.range.start + end + 1);
            if let Some(marker) = marker {
                formatting.callouts.push(Callout {
                    range: frame.range.clone(),
                    kind: match kind {
                        BlockQuoteKind::Note => CalloutKind::Note,
                        BlockQuoteKind::Tip => CalloutKind::Tip,
                        BlockQuoteKind::Important => CalloutKind::Important,
                        BlockQuoteKind::Warning => CalloutKind::Warning,
                        BlockQuoteKind::Caution => CalloutKind::Caution,
                    },
                    marker,
                });
            }
        }
        _ => {}
    }
}

/// Where a link of `link_type` to `destination` leads another app to, if
/// it does: to a web page or a mail address, not to a file of the vault.
fn web_url(link_type: LinkType, destination: &str) -> Option<String> {
    if link_type == LinkType::Email {
        return Some(format!("mailto:{destination}"));
    }
    // A scheme, like `https:`, before any `/`.
    let scheme = &destination[..destination.find(':')?];
    let has_scheme = scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    has_scheme.then(|| destination.to_owned())
}

/// How deep a list item inside the elements of `stack` is nested, from 1.
fn depth(stack: &[Frame]) -> u8 {
    let lists = stack
        .iter()
        .filter(|frame| matches!(frame.tag, Tag::List(_)))
        .count();
    u8::try_from(lists).unwrap_or(u8::MAX)
}

/// The marker of `frame` if it is an item of a bullet list, inside the
/// elements of `stack`.
fn list_marker(
    text: &str,
    frame: &Frame,
    stack: &[Frame],
) -> Option<(Range<usize>, MarkdownStyle)> {
    let list = stack.last()?;
    if !matches!(frame.tag, Tag::Item) {
        return None;
    }
    let gap = gaps(frame.range.clone(), &frame.children)
        .into_iter()
        .next()?;
    let marker = trim(text, gap)?;
    let content = &text[marker.clone()];
    let style = match list.tag {
        Tag::List(None) => {
            // Only once a space follows, so that a bullet does not appear
            // over the cursor while it is being typed.
            let is_bullet =
                matches!(content, "-" | "*" | "+") && text[marker.end..].starts_with([' ', '\t']);
            is_bullet.then(|| MarkdownStyle::Bullet(depth(stack)))?
        }
        Tag::List(Some(_)) => {
            let is_number = content.strip_suffix(['.', ')']).is_some_and(|digits| {
                !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
            });
            is_number.then_some(MarkdownStyle::Number)?
        }
        _ => return None,
    };
    Some((marker, style))
}

/// Styles the element of `frame`, and returns the inline elements whose
/// markup is hidden: the element itself, or backslashes that escape
/// characters in it.
fn style_element(
    text: &str,
    frame: &Frame,
    mode: MarkdownMode,
    in_quote: bool,
    styles: &mut Vec<(Range<usize>, MarkdownStyle)>,
) -> Vec<InlineMarkup> {
    let range = frame.range.clone();
    let style = match &frame.tag {
        Tag::Strong => Some(MarkdownStyle::Strong),
        Tag::Emphasis => Some(MarkdownStyle::Emphasis),
        Tag::Strikethrough => Some(MarkdownStyle::Strikethrough),
        Tag::CodeBlock(_) => Some(MarkdownStyle::CodeBlock),
        Tag::BlockQuote(_) => Some(MarkdownStyle::Quote),
        Tag::TableHead => Some(MarkdownStyle::TableHeader),
        Tag::Heading { .. } if mode == MarkdownMode::Block => return Vec::new(),
        Tag::Heading { level, .. } => Some(MarkdownStyle::Heading(*level as u8)),
        // Front matter is not evaluated, so all of it is dimmed.
        Tag::MetadataBlock(_) => {
            styles.extend(trim(text, range).map(|range| (range, MarkdownStyle::Markup)));
            return Vec::new();
        }
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
    let mut markup = Vec::new();
    let mut escapes = Vec::new();
    for gap in gaps(range.clone(), &frame.children) {
        if let Some(trimmed) = trim(text, gap.clone()) {
            // A backslash that escapes the character after it.
            let is_escape = &text[trimmed.clone()] == "\\"
                && text[trimmed.end..].starts_with(|c: char| c.is_ascii_punctuation());
            if is_escape {
                escapes.push(InlineMarkup {
                    element: trimmed.start..trimmed.end + 1,
                    markup: vec![trimmed.clone()],
                });
            }
            if in_quote {
                styles.extend(
                    quote_markers(text, &trimmed)
                        .map(|marker| (marker, MarkdownStyle::QuoteMarker)),
                );
            }
            styles.push((trimmed.clone(), MarkdownStyle::Markup));
            markup.push(if gap.start == range.start {
                // With the space after `#`, which would indent the title.
                let spaces = &text[trimmed.end..gap.end];
                trimmed.start
                    ..trimmed.end + spaces.len() - spaces.trim_start_matches([' ', '\t']).len()
            } else {
                trimmed
            });
        }
    }
    let is_inline = match frame.tag {
        Tag::Strong | Tag::Emphasis | Tag::Strikethrough | Tag::Link { .. } => true,
        Tag::Heading { .. } if text[range.start..].starts_with('#') => true,
        // Setext headings have their markup on a line of its own, which
        // goes with its line break.
        Tag::Heading { .. } => {
            markup = markup
                .into_iter()
                .map(|line| line_start(text, line.start)..next_line_start(text, line.start))
                .collect();
            true
        }
        _ => false,
    };
    if is_inline
        && !markup.is_empty()
        && let Some(element) = trim(text, range)
    {
        escapes.push(InlineMarkup { element, markup });
    }
    escapes
}

/// The `>` at the start of the lines of `range`, in a quote. Not those in
/// other markup, like the end of an autolink `<https://example.com>`.
fn quote_markers<'a>(
    text: &'a str,
    range: &Range<usize>,
) -> impl Iterator<Item = Range<usize>> + 'a {
    let mut start = range.start;
    text[range.clone()]
        .split_inclusive('\n')
        .filter_map(move |line| {
            let line_range = start..start + line.len();
            start += line.len();
            // Only where nothing but other markers is before it on its line.
            let line_start = text[..line_range.start]
                .rfind('\n')
                .map_or(0, |end| end + 1);
            if !text[line_start..line_range.start]
                .chars()
                .all(|c| c == '>' || c.is_whitespace())
            {
                return None;
            }
            let markers = line.len()
                - line
                    .trim_start_matches(|c: char| c == '>' || c.is_whitespace())
                    .len();
            trim(text, line_range.start..line_range.start + markers)
        })
}

/// The byte ranges of the lines of `text`, without their line breaks.
fn lines(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    text.split_inclusive('\n').map(move |line| {
        let range = start..start + line.trim_end_matches(['\n', '\r']).len();
        start += line.len();
        range
    })
}

/// Where the line with `at` starts.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |end| end + 1)
}

/// Where the line with `at` ends, after its line break.
fn next_line_start(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |end| at + end + 1)
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

const OPTIONS: Options = Options::ENABLE_STRIKETHROUGH
    .union(Options::ENABLE_TABLES)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_GFM);

fn options(mode: MarkdownMode) -> Options {
    match mode {
        MarkdownMode::Block => OPTIONS,
        // Project notes may start with front matter and link to each other.
        MarkdownMode::Full => {
            OPTIONS | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS | Options::ENABLE_WIKILINKS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MarkdownStyle::*;

    /// The styled parts of `text`, as text for readable assertions.
    fn styled(text: &str, mode: MarkdownMode) -> Vec<(&str, MarkdownStyle)> {
        markdown_formatting(text, mode)
            .styles
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
    fn escaped_headings_dim_their_backslash() {
        let text = escape_headings("# Title\n\nSetext\n---\n");
        assert_eq!(
            styled(&text, MarkdownMode::Block),
            [("\\", Markup), ("\\", Markup)]
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
                ("-", Bullet(1)),
                ("-", Markup),
                ("-", Bullet(1)),
                ("[x]", Markup),
                ("> quoted\n> more\n", Quote),
                (">", QuoteMarker),
                (">", Markup),
                (">", QuoteMarker),
                (">", Markup),
                ("```rust\nlet a = 1;\n```", CodeBlock),
                ("```rust", Markup),
                ("```", Markup),
            ]
        );
    }

    /// The inline elements of `text` with their markup, as text.
    fn inline(text: &str, mode: MarkdownMode) -> Vec<(&str, Vec<&str>)> {
        markdown_formatting(text, mode)
            .inline_markup
            .into_iter()
            .map(|inline| {
                let markup = inline
                    .markup
                    .into_iter()
                    .map(|range| &text[range])
                    .collect();
                (&text[inline.element], markup)
            })
            .collect()
    }

    #[test]
    fn inline_elements_and_their_markup() {
        let text = "## Title\n\n**a** _b_ `c` ~~d~~ [e](f) <https://g.h>\n\nSetext\n===\n";
        assert_eq!(
            inline(text, MarkdownMode::Full),
            [
                ("## Title", vec!["## "]),
                ("**a**", vec!["**", "**"]),
                ("_b_", vec!["_", "_"]),
                ("`c`", vec!["`", "`"]),
                ("~~d~~", vec!["~~", "~~"]),
                ("[e](f)", vec!["[", "](f)"]),
                ("<https://g.h>", vec!["<", ">"]),
                ("Setext\n===", vec!["===\n"]),
            ]
        );
        // Block Markdown has no headings.
        assert_eq!(inline("# Title", MarkdownMode::Block), []);
    }

    #[test]
    fn web_links() {
        let text = "[a](https://example.com) <anna@example.org> [b](notes/x.md) \
                    <https://c.d> [e](#top) [[x/y]]";
        let formatting = markdown_formatting(text, MarkdownMode::Full);
        let links: Vec<(&str, &str)> = formatting
            .web_links
            .iter()
            .map(|link| (&text[link.range.clone()], link.url.as_str()))
            .collect();
        assert_eq!(
            links,
            [
                ("[a](https://example.com)", "https://example.com"),
                ("<anna@example.org>", "mailto:anna@example.org"),
                ("<https://c.d>", "https://c.d"),
            ]
        );
    }

    #[test]
    fn callouts_hide_their_quote_markers() {
        let text = "> [!NOTE]\n> Text\n";
        let markers: Vec<_> = styled(text, MarkdownMode::Block)
            .into_iter()
            .filter(|(_, style)| *style == QuoteMarker)
            .collect();
        assert_eq!(markers, [(">", QuoteMarker); 2]);
    }

    #[test]
    fn callouts() {
        let text = "> [!WARNING]\n> Careful\n\n> [!tip]\n> Easy\n\n> Plain\n";
        let callouts: Vec<(CalloutKind, &str)> = markdown_formatting(text, MarkdownMode::Block)
            .callouts
            .into_iter()
            .map(|callout| (callout.kind, &text[callout.marker]))
            .collect();
        assert_eq!(
            callouts,
            [
                (CalloutKind::Warning, "[!WARNING]"),
                (CalloutKind::Tip, "[!tip]")
            ]
        );
    }

    #[test]
    fn escapes_and_setext_headings_hide_their_markup() {
        assert_eq!(
            inline("a \\* b \\ c", MarkdownMode::Block),
            [("\\*", vec!["\\"])]
        );
        assert_eq!(
            inline("Title\n===\n\nText", MarkdownMode::Full),
            [("Title\n===", vec!["===\n"])]
        );
    }

    #[test]
    fn tasks() {
        let text = "- [ ] Call Anna\n  1. [x] Done  \n- [ ]\n- [y] no task\n";
        let tasks: Vec<(&str, &str, bool, &str)> = markdown_formatting(text, MarkdownMode::Block)
            .tasks
            .into_iter()
            .map(|task| {
                (
                    &text[task.marker],
                    &text[task.check_box],
                    task.done,
                    &text[task.text],
                )
            })
            .collect();
        assert_eq!(
            tasks,
            [
                ("-", "[ ]", false, "Call Anna"),
                ("1.", "[x]", true, "Done"),
                ("-", "[ ]", false, ""),
            ]
        );
    }

    #[test]
    fn bullets_know_their_depth() {
        let text = "* a\n  + b\n    1. c\n       - d\n";
        let bullets: Vec<_> = styled(text, MarkdownMode::Block)
            .into_iter()
            .filter(|(_, style)| matches!(style, Bullet(_)))
            .collect();
        assert_eq!(
            bullets,
            [("*", Bullet(1)), ("+", Bullet(2)), ("-", Bullet(4))]
        );
    }

    #[test]
    fn list_markers() {
        let text = "3. a\n4. b\n   1) c\n\n- e\n";
        let markers: Vec<_> = styled(text, MarkdownMode::Block)
            .into_iter()
            .filter(|(_, style)| matches!(style, Number | Bullet(_)))
            .collect();
        assert_eq!(
            markers,
            [
                ("3.", Number),
                ("4.", Number),
                ("1)", Number),
                ("-", Bullet(1))
            ]
        );
    }

    #[test]
    fn autolinks_in_quotes_keep_their_brackets() {
        let markers: Vec<_> = styled("> see <https://a.b>\n>> nested", MarkdownMode::Block)
            .into_iter()
            .filter(|(_, style)| *style == QuoteMarker)
            .collect();
        // The outer and the nested quote each have a marker.
        assert_eq!(markers, [(">", QuoteMarker); 3]);
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
    fn front_matter_is_markup_in_notes() {
        let text = "---\naliases: [\"PSP\"]\n---\n\nText\n";
        assert_eq!(
            styled(text, MarkdownMode::Full),
            [("---\naliases: [\"PSP\"]\n---", Markup)]
        );
    }

    #[test]
    fn wiki_links_in_notes() {
        let text = "[[infra/deployment]] and [[webshop/checkout-flow|checkout]]";
        assert_eq!(
            styled(text, MarkdownMode::Full),
            [
                ("[[", Markup),
                ("infra/deployment", Link),
                ("]]", Markup),
                ("[[webshop/checkout-flow|", Markup),
                ("checkout", Link),
                ("]]", Markup),
            ]
        );
        assert_eq!(styled(text, MarkdownMode::Block), []);
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
            [("***", Rule), ("***", Markup)]
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
