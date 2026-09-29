//! Mermaid diagrams: code blocks in `mermaid` are drawn as their diagram
//! while the cursor is elsewhere, but as code while it is in them or when
//! the diagram cannot be drawn. Diagrams are drawn in the background and
//! kept while the text has them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use knotbook_core::Formatting;
use mermaid_rs_renderer::{RenderOptions, Theme};
use resvg::{tiny_skia, usvg};

use super::decorations::{Decorations, line_span, text_edges};
use super::styling::Styling;
use super::{CORNER_RADIUS, tags};

/// The language of the code blocks drawn as diagrams.
const LANGUAGE: &str = "mermaid";
/// Shrinks the line breaks of code blocks drawn as diagrams.
const LINE_BREAK_TAG: &str = "diagram-line-break";
/// Leaves line breaks about a pixel high.
const LINE_BREAK_SCALE: f64 = 0.05;
/// The space above and below a diagram.
const PADDING: i32 = 8;
/// That of the app, rather than that of Mermaid.
const FONT_FAMILY: &str = "\"Adwaita Sans\", Cantarell, sans-serif";
const FONT_SIZE: f32 = 14.0;
/// The longest side of an image, in pixels, beyond which diagrams are
/// drawn less sharp.
const MAX_PIXELS: f32 = 8192.0;

/// The fonts of the system, loaded once, when the first diagram is drawn.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    Arc::new(fonts)
});

/// The code of a diagram and how to draw it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Request {
    code: String,
    dark: bool,
    /// The scale factor of the view.
    scale: i32,
}

/// A diagram drawn as an image, as sent from the thread that draws it.
#[derive(Debug)]
pub(super) struct Image {
    pixels: Vec<u8>,
    pixel_width: u32,
    pixel_height: u32,
    /// In pixels of the view.
    width: f32,
    height: f32,
}

/// A diagram ready to be drawn.
#[derive(Debug)]
pub(super) struct Diagram {
    texture: gdk::Texture,
    /// In pixels of the view, as large as Mermaid draws it.
    width: f32,
    height: f32,
}

#[derive(Debug)]
enum State {
    Pending,
    Failed,
    Done(Rc<Diagram>),
}

/// The diagrams of the text as last styled, by their code.
#[derive(Debug, Default)]
pub(super) struct Diagrams(RefCell<HashMap<Request, State>>);

/// A code block drawn as its diagram, in characters.
#[derive(Debug)]
pub(super) struct DiagramCard {
    /// Of the whole code block.
    pub(super) range: Range<i32>,
    /// The start of the line of the opening fence, where the diagram is.
    line: i32,
    diagram: Rc<Diagram>,
    /// How high the diagram is drawn, the room for it in the text.
    height: i32,
}

/// Draws the Mermaid code blocks of `formatting` as diagrams, but for the
/// one the cursor is in and those whose diagram is not ready or cannot be
/// drawn, and adds their cards to `decorations`. Returns the diagrams to
/// draw, which `Diagrams::finish` takes when they are ready.
pub(super) fn style(
    styling: &Styling,
    formatting: &Formatting,
    decorations: &mut Decorations,
    diagrams: &Diagrams,
) -> Vec<Request> {
    let dark = adw::StyleManager::default().is_dark();
    let scale = styling.view.scale_factor();
    let mut known = diagrams.0.take();
    let mut kept = HashMap::new();
    let mut missing = Vec::new();
    for block in &formatting.code_blocks {
        let Some(opening) = block.fences.first() else {
            continue;
        };
        if !block.language.eq_ignore_ascii_case(LANGUAGE) || block.code.is_empty() {
            continue;
        }
        let request = Request {
            code: styling.text[block.code.clone()].to_owned(),
            dark,
            scale,
        };
        // Not drawn while it is being edited, but kept in case it is left
        // as it was.
        if styling.is_at(&block.range) {
            if let Some(state) = known.remove(&request) {
                kept.insert(request, state);
            }
            continue;
        }
        let state = kept.entry(request.clone()).or_insert_with(|| {
            known.remove(&request).unwrap_or_else(|| {
                missing.push(request);
                State::Pending
            })
        });
        let State::Done(diagram) = state else {
            continue;
        };
        let height = diagram.fit(styling.view).1.round() as i32;
        let end = block
            .fences
            .last()
            .map_or(block.range.end, |fence| fence.end)
            .max(block.range.end);
        conceal(styling, opening, end, height);
        decorations.diagrams.push(DiagramCard {
            range: styling.chars(&block.range),
            line: styling.offset(opening.start),
            diagram: Rc::clone(diagram),
            height,
        });
    }
    diagrams.0.replace(kept);
    missing
}

/// Hides the code block from the `opening` fence to `end`, and makes room
/// for a diagram `height` high above it.
fn conceal(styling: &Styling, opening: &Range<usize>, end: usize, height: i32) {
    let buffer = &styling.buffer;
    // Last, so that it wins over the other tags.
    tags::get_or_add(buffer, LINE_BREAK_TAG, || {
        gtk::TextTag::builder()
            .name(LINE_BREAK_TAG)
            .foreground_rgba(&tags::INVISIBLE)
            .scale(LINE_BREAK_SCALE)
            .build()
    });
    // Line breaks stay, but shrunk: after lines hidden with their line
    // breaks, GTK takes the pointer for a place off the end of a line and
    // aborts.
    let mut start = opening.start;
    for line in styling.text[opening.start..end].split_inclusive('\n') {
        let line_end = start + line.len();
        let text_end = start + line.trim_end_matches(['\r', '\n']).len();
        styling.tag(tags::HIDDEN, &(start..text_end));
        styling.tag(LINE_BREAK_TAG, &(text_end..line_end));
        start = line_end;
    }
    let name = format!("diagram {height}");
    tags::get_or_add(buffer, &name, || {
        gtk::TextTag::builder()
            .name(&name)
            .pixels_above_lines(PADDING + height + PADDING)
            .build()
    });
    let line = styling.offset(opening.start);
    tags::apply_to_lines(buffer, &name, line..line);
}

impl Diagrams {
    /// Keeps the diagram `image` drawn for `request`, or that it cannot be
    /// drawn. Returns whether the text still has it.
    pub(super) fn finish(&self, request: Request, image: Option<Image>) -> bool {
        let mut states = self.0.borrow_mut();
        let Some(state @ State::Pending) = states.get_mut(&request) else {
            return false;
        };
        *state = match image {
            Some(image) => State::Done(Rc::new(Diagram::new(image))),
            None => State::Failed,
        };
        true
    }
}

impl Request {
    /// Draws the diagram, unless its code is no diagram Mermaid knows.
    /// Slow: the first diagram loads the fonts of the system.
    pub(super) fn render(&self) -> Option<Image> {
        let mut theme = if self.dark {
            Theme::dark()
        } else {
            Theme::modern()
        };
        theme.font_family = FONT_FAMILY.to_owned();
        theme.font_size = FONT_SIZE;
        let options = RenderOptions {
            theme,
            ..RenderOptions::default()
        };
        let svg = mermaid_rs_renderer::render_with_options(&self.code, options).ok()?;
        let options = usvg::Options {
            font_family: "sans-serif".to_owned(),
            fontdb: Arc::clone(&FONTS),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options).ok()?;
        let size = tree.size();
        let (width, height) = (size.width(), size.height());
        let scale = (self.scale as f32).min(MAX_PIXELS / width.max(height));
        let pixel_width = (width * scale).ceil() as u32;
        let pixel_height = (height * scale).ceil() as u32;
        let mut pixmap = tiny_skia::Pixmap::new(pixel_width, pixel_height)?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        Some(Image {
            pixels: pixmap.take(),
            pixel_width,
            pixel_height,
            width,
            height,
        })
    }
}

impl Diagram {
    fn new(image: Image) -> Self {
        let texture = gdk::MemoryTexture::new(
            image.pixel_width as i32,
            image.pixel_height as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_owned(image.pixels),
            image.pixel_width as usize * 4,
        );
        Self {
            texture: texture.upcast(),
            width: image.width,
            height: image.height,
        }
    }

    /// How wide and high the diagram is drawn in `view`: as large as
    /// Mermaid draws it, but no wider than the text.
    fn fit(&self, view: &gtk::TextView) -> (f32, f32) {
        let (left, right) = text_edges(view, &view.visible_rect());
        let available = right - left;
        // Before the view has a size, it has no room.
        let scale = if available > 0.0 {
            (available / self.width).min(1.0)
        } else {
            1.0
        };
        (self.width * scale, self.height * scale)
    }
}

impl DiagramCard {
    /// Whether the diagram has another height in `view` than it has room
    /// for, as the view changed its width.
    pub(super) fn misfits(&self, view: &gtk::TextView) -> bool {
        self.diagram.fit(view).1.round() as i32 != self.height
    }

    /// Draws the diagram in the middle of the text, in buffer coordinates.
    pub(super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let Some((top, _)) = line_span(view, &(self.line..self.line + 1), visible) else {
            return;
        };
        let (left, right) = text_edges(view, visible);
        let (width, height) = self.diagram.fit(view);
        let x = left + ((right - left - width) / 2.0).round();
        let bounds = graphene::Rect::new(x, top + PADDING as f32, width, height);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, CORNER_RADIUS));
        snapshot.append_texture(&self.diagram.texture, &bounds);
        snapshot.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(code: &str) -> Request {
        Request {
            code: code.to_owned(),
            dark: false,
            scale: 2,
        }
    }

    #[test]
    fn diagrams_are_drawn_at_the_scale_of_the_view() {
        let image = request("flowchart LR\n  A[Start] --> B{Choice}\n  B --> C\n")
            .render()
            .expect("the diagram is valid");
        assert!(image.width > 0.0 && image.height > 0.0);
        assert_eq!(image.pixel_width, (image.width * 2.0).ceil() as u32);
        assert_eq!(
            image.pixels.len(),
            (image.pixel_width * image.pixel_height * 4) as usize
        );
    }

    #[test]
    fn code_that_is_no_diagram_is_not_drawn() {
        assert!(request("this is no diagram").render().is_none());
    }
}
