//! Callouts like `> [!NOTE]`: quotes on a card in the colour of their kind,
//! titled with it instead of their marker while the cursor is elsewhere.

use std::ops::Range;

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{gdk, graphene, gsk, pango};
use knotbook_core::{CalloutKind, Formatting};

use super::decorations::{Decorations, QUOTE_BAR_WIDTH, char_location, line_span, text_edges};
use super::styling::Styling;
use super::{CORNER_RADIUS, tags};
use crate::colors::with_alpha;

/// The background of callouts, as the alpha of their colour.
const BACKGROUND_ALPHA: f32 = 0.1;

/// A callout drawn as a card, in characters.
#[derive(Debug)]
pub(super) struct CalloutCard {
    range: Range<i32>,
    kind: CalloutKind,
    /// Where its title is drawn, over its hidden marker, unless the marker
    /// is edited.
    title: Option<i32>,
}

/// Styles the callouts of `formatting` and adds their cards to
/// `decorations`, instead of the bars of their quotes.
pub(super) fn style(styling: &Styling, formatting: &Formatting, decorations: &mut Decorations) {
    for callout in &formatting.callouts {
        let range = styling.chars(&callout.range);
        styling.tag(tags::CALLOUT, &callout.range);
        decorations.quotes.retain(|quote| *quote != range);
        decorations.revealable.push(styling.chars(&callout.marker));
        let title = (!styling.is_at(&callout.marker)).then(|| {
            styling.tag(tags::CONCEALED, &callout.marker);
            styling.offset(callout.marker.start)
        });
        decorations.callouts.push(CalloutCard {
            range,
            kind: callout.kind,
            title,
        });
    }
}

impl CalloutCard {
    /// Draws the card with its bar and title, in buffer coordinates.
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
        let (color, title) = look(self.kind);
        let card = graphene::Rect::new(left, top, right - left, bottom - top);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(card, CORNER_RADIUS));
        snapshot.append_color(&with_alpha(&color, BACKGROUND_ALPHA), &card);
        let bar = graphene::Rect::new(left, top, QUOTE_BAR_WIDTH, bottom - top);
        snapshot.append_color(&color, &bar);
        snapshot.pop();
        if let Some(at) = self.title {
            let location = char_location(view, at);
            let layout = view.create_pango_layout(Some(&title));
            let attributes = pango::AttrList::new();
            attributes.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
            layout.set_attributes(Some(&attributes));
            let height = layout.pixel_size().1;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                location.x() as f32,
                location.y() as f32 + (location.height() - height) as f32 / 2.0,
            ));
            snapshot.append_layout(&layout, &color);
            snapshot.restore();
        }
    }
}

/// The colour and title of callouts of `kind`, from the palette of GNOME.
fn look(kind: CalloutKind) -> (gdk::RGBA, String) {
    let is_dark = adw::StyleManager::default().is_dark();
    let (light, dark, title) = match kind {
        CalloutKind::Note => ("#1c71d8", "#78aeed", gettext("Note")),
        CalloutKind::Tip => ("#26a269", "#8ff0a4", gettext("Tip")),
        CalloutKind::Important => ("#813d9c", "#dc8add", gettext("Important")),
        CalloutKind::Warning => ("#c64600", "#ffa348", gettext("Warning")),
        CalloutKind::Caution => ("#c01c28", "#f66151", gettext("Caution")),
    };
    let color =
        gdk::RGBA::parse(if is_dark { dark } else { light }).expect("the colours are valid");
    (color, title)
}
