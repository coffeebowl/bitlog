//! The text of a view while it is styled, which the modules that style its
//! parts share.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gtk::pango;
use gtk::prelude::*;

use super::applied_tags::Tagged;

/// How many widths `Widths` keeps at most: those of the cells of all tables
/// of a long text, and the parts of those cut off.
const WIDTHS_SIZE: usize = 50_000;

/// How wide texts are shown in the font of a view, in Pango units, by
/// their text and attributes, as measured: tables measure all their cells
/// on every change.
pub(super) type Widths = HashMap<(String, String), i32>;

/// The text of a view, with where its cursor is, and the tags that style
/// it. The core counts bytes, text buffers count characters: ranges of the
/// text are in bytes, unless they are said to be characters.
pub(super) struct Styling<'a> {
    pub(super) view: &'a gtk::TextView,
    pub(super) buffer: gtk::TextBuffer,
    pub(super) text: &'a str,
    /// The character offset of each byte offset of the text.
    offsets: &'a [i32],
    /// Where the cursor is, as a character offset, while the user may be
    /// editing.
    pub(super) cursor: Option<i32>,
    widths: &'a RefCell<Widths>,
    tag_table: gtk::TextTagTable,
    /// The tags to apply, see `MarkdownView::restyle`.
    tags: RefCell<Vec<Tagged>>,
}

impl<'a> Styling<'a> {
    pub(super) fn new(
        view: &'a gtk::TextView,
        text: &'a str,
        offsets: &'a [i32],
        cursor: Option<i32>,
        widths: &'a RefCell<Widths>,
    ) -> Self {
        let buffer = view.buffer();
        Self {
            view,
            tag_table: buffer.tag_table(),
            buffer,
            text,
            offsets,
            cursor,
            widths,
            tags: RefCell::default(),
        }
    }

    /// The character offset of byte `offset` of the text.
    pub(super) fn offset(&self, offset: usize) -> i32 {
        self.offsets[offset]
    }

    /// The characters of `range` of the text.
    pub(super) fn chars(&self, range: &Range<usize>) -> Range<i32> {
        self.offsets[range.start]..self.offsets[range.end]
    }

    /// Applies the tag named `name` to `range` of the text.
    pub(super) fn tag(&self, name: &str, range: &Range<usize>) {
        self.tag_chars(name, self.chars(range));
    }

    /// Applies the tag named `name` to the characters of `range`.
    pub(super) fn tag_chars(&self, name: &str, range: Range<i32>) {
        let tag = self
            .tag_table
            .lookup(name)
            .expect("tags are added before they are applied");
        self.tags.borrow_mut().push((tag, range));
    }

    /// Applies the tag named `name` to the whole lines of the characters
    /// of `range`, with their line breaks, as tags that format lines need.
    pub(super) fn tag_lines(&self, name: &str, range: Range<i32>) {
        let mut start = self.buffer.iter_at_offset(range.start);
        start.set_line_offset(0);
        let mut end = self.buffer.iter_at_offset(range.end);
        if range.is_empty() || !end.starts_line() {
            end.forward_line();
        }
        self.tag_chars(name, start.offset()..end.offset());
    }

    /// The tags to apply.
    pub(super) fn into_tags(self) -> Vec<Tagged> {
        self.tags.into_inner()
    }

    /// Whether the cursor is in `range` of the text, or at its end.
    pub(super) fn is_at(&self, range: &Range<usize>) -> bool {
        holds(&self.chars(range), self.cursor)
    }

    /// How wide `text` is shown in the font of the view with `attributes`,
    /// in Pango units.
    pub(super) fn text_width(&self, text: &str, attributes: &pango::AttrList) -> i32 {
        let key = (text.to_owned(), attributes.to_str().into());
        if let Some(width) = self.widths.borrow().get(&key) {
            return *width;
        }
        let layout = self.view.create_pango_layout(Some(text));
        layout.set_attributes(Some(attributes));
        let width = layout.size().0;
        let mut widths = self.widths.borrow_mut();
        if widths.len() >= WIDTHS_SIZE {
            widths.clear();
        }
        widths.insert(key, width);
        width
    }

    /// How wide `part` is shown in the font of the view, in whole pixels.
    pub(super) fn width(&self, part: &str) -> i32 {
        whole_pixels(self.text_width(part, &pango::AttrList::new()))
    }
}

/// `units` of Pango in whole pixels, rounded up.
pub(super) fn whole_pixels(units: i32) -> i32 {
    (units + pango::SCALE - 1) / pango::SCALE
}

/// Whether there is a `cursor` and it is in the characters of `range`, or
/// at their end.
pub(super) fn holds(range: &Range<i32>, cursor: Option<i32>) -> bool {
    cursor.is_some_and(|cursor| (range.start..=range.end).contains(&cursor))
}
