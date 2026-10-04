//! The text of a view while it is styled, which the modules that style its
//! parts share.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use gtk::pango;
use gtk::prelude::*;

use super::tags;

/// The text of a view, with where its cursor is. The core counts bytes,
/// text buffers count characters: ranges of the text are in bytes, unless
/// they are said to be characters.
pub(super) struct Styling<'a> {
    pub(super) view: &'a gtk::TextView,
    pub(super) buffer: gtk::TextBuffer,
    pub(super) text: &'a str,
    /// The character offset of each byte offset of the text.
    offsets: &'a [i32],
    /// Where the cursor is, as a character offset, while the user may be
    /// editing.
    pub(super) cursor: Option<i32>,
    /// How wide parts of the text are shown, as measured: list markers and
    /// indents are mostly the same few.
    widths: RefCell<HashMap<String, i32>>,
}

impl<'a> Styling<'a> {
    pub(super) fn new(
        view: &'a gtk::TextView,
        text: &'a str,
        offsets: &'a [i32],
        cursor: Option<i32>,
    ) -> Self {
        Self {
            view,
            buffer: view.buffer(),
            text,
            offsets,
            cursor,
            widths: RefCell::default(),
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
        tags::apply(&self.buffer, name, self.chars(range));
    }

    /// Whether the cursor is in `range` of the text, or at its end.
    pub(super) fn is_at(&self, range: &Range<usize>) -> bool {
        holds(&self.chars(range), self.cursor)
    }

    /// `part` in the font of the view.
    pub(super) fn layout(&self, part: &str) -> pango::Layout {
        self.view.create_pango_layout(Some(part))
    }

    /// How wide `part` is shown in the font of the view, in pixels.
    pub(super) fn width(&self, part: &str) -> i32 {
        if let Some(width) = self.widths.borrow().get(part) {
            return *width;
        }
        let width = self.layout(part).pixel_size().0;
        self.widths.borrow_mut().insert(part.to_owned(), width);
        width
    }
}

/// Whether there is a `cursor` and it is in the characters of `range`, or
/// at their end.
pub(super) fn holds(range: &Range<i32>, cursor: Option<i32>) -> bool {
    cursor.is_some_and(|cursor| (range.start..=range.end).contains(&cursor))
}
