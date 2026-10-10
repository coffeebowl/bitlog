//! Code blocks, drawn as cards: while the cursor is elsewhere without their
//! fences, but with their language in their top right corner.

use std::ops::Range;

use bitlog_core::Formatting;
use gtk::prelude::*;
use gtk::{gdk, graphene};

use super::decorations::{Decorations, line_span, text_edges};
use super::styling::Styling;
use super::{CODE_ALPHA, CORNER_RADIUS, MARKUP_ALPHA, tags};
use crate::colors::with_alpha;
use crate::drawing::{self, append_layout, fill_rounded};

/// The language of a code block, in its top right corner.
const LABEL_SCALE: f64 = 0.75;
const LABEL_PADDING: f32 = 8.0;

/// A code block drawn as a card, in characters.
#[derive(Debug)]
pub(super) struct CodeCard {
    range: Range<i32>,
    /// Shown while its fences are hidden.
    language: Option<String>,
}

/// Hides the fences of the code blocks of `formatting`, but for the one
/// the cursor is in, and adds their cards to `decorations`, but for those
/// drawn as diagrams.
pub(super) fn style(styling: &Styling, formatting: &Formatting, decorations: &mut Decorations) {
    for block in &formatting.code_blocks {
        let mut card = CodeCard {
            range: styling.chars(&block.range),
            language: None,
        };
        if !block.fences.is_empty() {
            decorations.revealable.push(card.range.clone());
        }
        if decorations
            .diagrams
            .iter()
            .any(|diagram| diagram.range == card.range)
        {
            continue;
        }
        // Without code, the block would be no more than its fences.
        let hides_fences =
            !block.fences.is_empty() && !block.code.is_empty() && !styling.is_at(&block.range);
        if hides_fences {
            for fence in &block.fences {
                styling.tag(tags::HIDDEN, fence);
            }
            // Space where the fences were, on the whole first and last line.
            let code = styling.chars(&block.code);
            tags::apply_to_lines(&styling.buffer, tags::CODE_TOP, code.start..code.start);
            tags::apply_to_lines(
                &styling.buffer,
                tags::CODE_BOTTOM,
                code.end - 1..code.end - 1,
            );
            card.language = (!block.language.is_empty()).then(|| block.language.clone());
        }
        decorations.code_blocks.push(card);
    }
}

impl CodeCard {
    /// Draws the card with its language, in buffer coordinates.
    pub(super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let Some((top, bottom)) = line_span(view, &self.range, visible) else {
            return;
        };
        let (left, right) = text_edges(view, visible);
        let color = view.color();
        let bounds = graphene::Rect::new(left, top, right - left, bottom - top);
        fill_rounded(
            snapshot,
            bounds,
            CORNER_RADIUS,
            &with_alpha(&color, CODE_ALPHA),
        );
        if let Some(language) = &self.language {
            let layout = drawing::layout(view, language, LABEL_SCALE, false);
            let corner = (right - LABEL_PADDING, top + LABEL_PADDING / 2.0);
            let label_color = with_alpha(&color, MARKUP_ALPHA);
            append_layout(snapshot, &layout, corner, (1.0, 0.0), &label_color);
        }
    }
}
