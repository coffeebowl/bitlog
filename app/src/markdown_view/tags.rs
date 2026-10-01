//! The tags that format the text: those of every style, made once, and
//! helpers for those made as they are needed.

use std::ops::Range;

use bitlog_core::MarkdownStyle;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, pango};

use super::{CODE_ALPHA, MARKUP_ALPHA};
use crate::colors::with_alpha;

const HEADING_SCALES: [f64; 6] = [1.6, 1.4, 1.25, 1.1, 1.0, 1.0];
/// Dims wiki links to notes that do not exist.
pub(super) const BROKEN_LINK: &str = "broken-link";
/// All of the text, with the default line height.
pub(super) const BODY: &str = "body";
/// Of all text, as a share of the height of the font.
const LINE_HEIGHT: f32 = 1.3;
/// Hides syntax that something else is drawn for, like bullets.
pub(super) const CONCEALED: &str = "concealed";
/// Hides syntax until the cursor is at it, taking no space.
pub(super) const HIDDEN: &str = "hidden";
/// Space above the first line of a code block without its fences.
pub(super) const CODE_TOP: &str = "code-top";
/// Space below the last line of a code block without its fences.
pub(super) const CODE_BOTTOM: &str = "code-bottom";
/// The numbers of ordered list items, in bold and not dimmed like other
/// syntax.
pub(super) const LIST_NUMBER: &str = "list-number";
/// Callouts are quotes, but not in italics.
pub(super) const CALLOUT: &str = "callout";
/// The text of tasks that are done.
pub(super) const TASK_DONE: &str = "task-done";
/// Text in no colour. Pango takes an alpha of 0 for none, which draws the
/// text opaque, so this is the least alpha it keeps.
pub(super) const INVISIBLE: gdk::RGBA = gdk::RGBA::new(0.0, 0.0, 0.0, 2.0 / 65535.0);
/// The space between a code block and its code.
const CODE_BLOCK_PADDING: i32 = 12;
/// The space above and below the code of a code block without its fences.
const CODE_BLOCK_VERTICAL_PADDING: i32 = 8;
/// The space between the bar of a quote and its hidden `>`.
const QUOTE_INDENT: i32 = 6;

/// Creates a tag for every style, and those that hide or space out text.
/// Tags created later win over earlier ones: the body tag comes first, the
/// markup tag after those of the other styles, and the list number and
/// concealed tags after it.
pub(super) fn create(buffer: &gtk::TextBuffer) {
    let monospace = || gtk::TextTag::builder().family("monospace");
    let bold = || gtk::TextTag::builder().weight(pango::Weight::Bold.into_glib());
    let italic = || gtk::TextTag::builder().style(pango::Style::Italic);
    let tags = [
        (MarkdownStyle::Strong, bold()),
        (MarkdownStyle::Emphasis, italic()),
        (
            MarkdownStyle::Strikethrough,
            gtk::TextTag::builder().strikethrough(true),
        ),
        (MarkdownStyle::Code, monospace()),
        (MarkdownStyle::CodeBlock, monospace()),
        (
            MarkdownStyle::Link,
            gtk::TextTag::builder().underline(pango::Underline::Single),
        ),
        (MarkdownStyle::Quote, italic()),
        (MarkdownStyle::TableHeader, bold()),
    ];
    let table = buffer.tag_table();
    // First, so that the others win over it.
    table.add(
        &gtk::TextTag::builder()
            .name(BODY)
            .line_height(LINE_HEIGHT)
            .build(),
    );
    for (style, builder) in tags {
        table.add(&builder.name(name(style)).build());
    }
    for (level, scale) in (1..).zip(HEADING_SCALES) {
        let tag = bold()
            .name(name(MarkdownStyle::Heading(level)))
            .scale(scale)
            .build();
        table.add(&tag);
    }
    // After the quote tag, so that it wins over it.
    table.add(
        &gtk::TextTag::builder()
            .name(CALLOUT)
            .style(pango::Style::Normal)
            .build(),
    );
    table.add(&gtk::TextTag::new(Some(BROKEN_LINK)));
    table.add(&gtk::TextTag::new(Some(&name(MarkdownStyle::Markup))));
    // After the markup tag, so that it wins over it.
    table.add(
        &gtk::TextTag::builder()
            .name(LIST_NUMBER)
            .weight(pango::Weight::Bold.into_glib())
            .build(),
    );
    let concealed = gtk::TextTag::builder()
        .name(CONCEALED)
        .foreground_rgba(&INVISIBLE)
        .build();
    table.add(&concealed);
    table.add(&gtk::TextTag::builder().name(HIDDEN).invisible(true).build());
    table.add(
        &gtk::TextTag::builder()
            .name(TASK_DONE)
            .strikethrough(true)
            .build(),
    );
    table.add(
        &gtk::TextTag::builder()
            .name(CODE_TOP)
            .pixels_above_lines(CODE_BLOCK_VERTICAL_PADDING)
            .build(),
    );
    table.add(
        &gtk::TextTag::builder()
            .name(CODE_BOTTOM)
            .pixels_below_lines(CODE_BLOCK_VERTICAL_PADDING)
            .build(),
    );
}

/// Fits the tags to the text colour of the theme, which dims Markdown
/// syntax, and to the margins of `view`.
pub(super) fn update(view: &gtk::TextView) {
    let table = view.buffer().tag_table();
    let tag = |name: &str| {
        table
            .lookup(name)
            .expect("the tags are created on construction")
    };
    let color = view.color();
    let dimmed = with_alpha(&color, MARKUP_ALPHA);
    tag(&name(MarkdownStyle::Markup)).set_foreground_rgba(Some(&dimmed));
    tag(BROKEN_LINK).set_foreground_rgba(Some(&dimmed));
    tag(&name(MarkdownStyle::Code)).set_background_rgba(Some(&with_alpha(&color, CODE_ALPHA)));
    tag(TASK_DONE).set_foreground_rgba(Some(&dimmed));
    tag(LIST_NUMBER).set_foreground_rgba(Some(&color));
    // Tags set margins from the edge, not from those of the view.
    let code_block = tag(&name(MarkdownStyle::CodeBlock));
    code_block.set_left_margin(view.left_margin() + CODE_BLOCK_PADDING);
    code_block.set_right_margin(view.right_margin() + CODE_BLOCK_PADDING);

    tag(&name(MarkdownStyle::Quote)).set_left_margin(view.left_margin() + QUOTE_INDENT);
}

pub(super) fn name(style: MarkdownStyle) -> String {
    match style {
        MarkdownStyle::Strong => "strong".to_owned(),
        MarkdownStyle::Emphasis => "emphasis".to_owned(),
        MarkdownStyle::Strikethrough => "strikethrough".to_owned(),
        MarkdownStyle::Code => "code".to_owned(),
        MarkdownStyle::CodeBlock => "code-block".to_owned(),
        MarkdownStyle::Link => "link".to_owned(),
        MarkdownStyle::Quote => "quote".to_owned(),
        MarkdownStyle::QuoteMarker | MarkdownStyle::Bullet(_) => CONCEALED.to_owned(),
        MarkdownStyle::TableHeader => "table-header".to_owned(),
        MarkdownStyle::Heading(level) => format!("heading-{level}"),
        MarkdownStyle::Number => LIST_NUMBER.to_owned(),
        // Dimmed like other syntax. The decorations hide the rules they
        // draw as lines.
        MarkdownStyle::Rule | MarkdownStyle::Markup => "markup".to_owned(),
    }
}

/// The tag named `name`, added by `build` unless it is there already. Tags
/// added later win over those of `create`.
pub(super) fn get_or_add(
    buffer: &gtk::TextBuffer,
    name: &str,
    build: impl FnOnce() -> gtk::TextTag,
) -> gtk::TextTag {
    let table = buffer.tag_table();
    table.lookup(name).unwrap_or_else(|| {
        let tag = build();
        table.add(&tag);
        tag
    })
}

/// The name of a tag that spaces characters out by `pixels`, added unless
/// it is there already. GTK takes no negative spacing, so less is none.
pub(super) fn spacing(buffer: &gtk::TextBuffer, pixels: i32) -> String {
    let pixels = pixels.max(0);
    let name = format!("spacing {pixels}");
    get_or_add(buffer, &name, || {
        gtk::TextTag::builder()
            .name(&name)
            .letter_spacing(pixels * pango::SCALE)
            .build()
    });
    name
}

/// The name of a tag that starts the lines a line wraps into `pixels`
/// further in than its first, added unless it is there already.
pub(super) fn hanging(buffer: &gtk::TextBuffer, pixels: i32) -> String {
    let name = format!("hanging {pixels}");
    // A negative indent indents all lines but the first.
    get_or_add(buffer, &name, || {
        gtk::TextTag::builder().name(&name).indent(-pixels).build()
    });
    name
}

/// Applies the tag named `name` to the whole lines of the characters of
/// `range`, with their line breaks, as tags that format lines need.
pub(super) fn apply_to_lines(buffer: &gtk::TextBuffer, name: &str, range: Range<i32>) {
    let mut start = buffer.iter_at_offset(range.start);
    start.set_line_offset(0);
    let mut end = buffer.iter_at_offset(range.end);
    if range.is_empty() || !end.starts_line() {
        end.forward_line();
    }
    buffer.apply_tag_by_name(name, &start, &end);
}

/// Applies the tag named `name` to the characters of `range`.
pub(super) fn apply(buffer: &gtk::TextBuffer, name: &str, range: Range<i32>) {
    buffer.apply_tag_by_name(
        name,
        &buffer.iter_at_offset(range.start),
        &buffer.iter_at_offset(range.end),
    );
}
