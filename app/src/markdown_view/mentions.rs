//! Mentions of known contacts like `@kim`: on a background like inline
//! code, followed by a click, and showing the contact's nickname or name
//! instead of `@` and their slug while the cursor is elsewhere. The file
//! keeps the slug.

use std::ops::Range;

use bitlog_core::{Formatting, mentions};
use gtk::gdk;
use gtk::prelude::*;

use super::decorations::Decorations;
use super::links::Target;
use super::styling::Styling;
use super::{Contacts, tags};
use crate::drawing::append_layout;

/// A name drawn over a mention, in characters.
#[derive(Debug)]
pub(super) struct MentionName {
    /// The first character of the slug, where the name starts.
    at: i32,
    name: String,
}

/// Styles the mentions of `contacts` in the text and adds them to
/// `decorations`. Returns them as links.
pub(super) fn style(
    styling: &Styling,
    formatting: &Formatting,
    decorations: &mut Decorations,
    contacts: &Contacts,
) -> Vec<(Range<i32>, Target)> {
    let mut links = Vec::new();
    for mention in mentions(styling.text) {
        let Some(contact) = contacts.contacts.iter().find(|c| c.slug == mention.contact) else {
            continue;
        };
        let span = mention.span;
        let shown = if styling.is_at(&span) || !is_plain(formatting, &span) {
            span.clone()
        } else {
            show_name(styling, decorations, &span, contact.display_name())
        };
        let (chars, shown) = (styling.chars(&span), styling.chars(&shown));
        decorations.chips.push(shown.clone());
        decorations.revealable.push(chars.clone());
        links.push((chars, Target::Contact(mention.contact, shown)));
    }
    links
}

/// Shows `name` instead of the mention at `span`. Returns what of it is
/// shown: the tiny characters at its ends are lower than the text.
fn show_name(
    styling: &Styling,
    decorations: &mut Decorations,
    span: &Range<usize>,
    name: &str,
) -> Range<usize> {
    // The first character of the slug is spaced out for the mention to be
    // as wide as the name, as GTK takes no negative spacing. Pango drops
    // half of the spacing at the start and end of a line, so the `@` and
    // the last character stay beside it, tiny. Only those between are
    // hidden. Slugs are ASCII, a byte each.
    let first = span.start + 1..span.start + 2;
    let last = (span.end - 1).max(first.end)..span.end;
    let width = styling.layout(name).size().0;
    let first_width = styling.layout(&styling.text[first.clone()]).size().0;
    let spacing = tags::spacing_units(&styling.buffer, width - first_width);
    styling.tag(&spacing, &first);
    styling.tag(tags::CONCEALED, span);
    styling.tag(tags::TINY, &(span.start..first.start));
    styling.tag(tags::TINY, &last);
    styling.tag(tags::HIDDEN, &(first.end..last.start));
    decorations.mention_names.push(MentionName {
        at: styling.offset(first.start),
        name: name.to_owned(),
    });
    first.start..last.start
}

/// Whether the mention at `span` is in text of no style, which the name
/// drawn over it would not have, like bold text, a quote or a heading, nor
/// in a table, whose columns line up by the text, or in a task done.
fn is_plain(formatting: &Formatting, span: &Range<usize>) -> bool {
    let holds = |range: &Range<usize>| range.start <= span.start && span.end <= range.end;
    !formatting.styles.iter().any(|(range, _)| holds(range))
        && !formatting
            .tables
            .iter()
            .flatten()
            .any(|line| holds(&line.range))
        && !formatting
            .tasks
            .iter()
            .any(|task| task.done && holds(&task.text))
}

impl MentionName {
    /// Draws the name over the concealed slug, on the baseline of the text,
    /// in buffer coordinates.
    pub(super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let iter = view.buffer().iter_at_offset(self.at);
        // As high as the font, without the space between lines.
        let (location, _) = view.cursor_locations(Some(&iter));
        if location.y() + location.height() < visible.y()
            || location.y() > visible.y() + visible.height()
        {
            return;
        }
        let layout = view.create_pango_layout(Some(&self.name));
        let start = (location.x() as f32, location.y() as f32);
        append_layout(snapshot, &layout, start, (0.0, 0.0), &view.color());
    }
}
