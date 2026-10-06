//! Mermaid diagrams: code blocks in `mermaid` are drawn as their diagram
//! while the cursor is elsewhere, but as code while it is in them or when
//! the diagram cannot be drawn. Diagrams are drawn in the background and
//! kept while the text has them, see `drawings`.

use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use bitlog_core::Formatting;
use gtk::prelude::*;
use gtk::{gdk, glib};
use mermaid_rs_renderer::{RenderOptions, Theme};
use resvg::{tiny_skia, usvg};

use super::decorations::Decorations;
use super::drawings::{Drawing, Drawings, LineMark, State};
use super::styling::Styling;
use super::tags;

/// The language of the code blocks drawn as diagrams.
const LANGUAGE: &str = "mermaid";
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

/// A code block drawn as its diagram.
#[derive(Debug)]
pub(super) struct DiagramCard {
    /// Of the whole code block, in characters.
    pub(super) range: Range<i32>,
    /// The line of the opening fence, where the diagram is.
    line: LineMark,
    diagram: Rc<Drawing>,
    /// How high the diagram is drawn, the room for it in the text.
    height: i32,
}

/// Draws the Mermaid code blocks of `formatting` as diagrams, but for the
/// one the cursor is in and those whose diagram is not ready or cannot be
/// drawn, and adds their cards to `decorations`. Returns the diagrams to
/// draw.
pub(super) fn style(
    styling: &Styling,
    formatting: &Formatting,
    decorations: &mut Decorations,
    diagrams: &Drawings<Request>,
) -> Vec<Request> {
    let dark = adw::StyleManager::default().is_dark();
    let scale = styling.view.scale_factor();
    let mut pass = diagrams.pass();
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
            pass.keep(request);
            continue;
        }
        let State::Done(diagram) = pass.state(request) else {
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
            line: LineMark::new(&styling.buffer, styling.offset(opening.start)),
            diagram,
            height,
        });
    }
    pass.end()
}

/// Hides the code block from the `opening` fence to `end`, and makes room
/// for a diagram `height` high above it.
fn conceal(styling: &Styling, opening: &Range<usize>, end: usize, height: i32) {
    let buffer = &styling.buffer;
    let mut start = opening.start;
    for line in styling.text[opening.start..end].split_inclusive('\n') {
        let line_end = start + line.len();
        tags::hide_line(styling, start..line_end);
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

impl Image {
    /// The diagram, ready to be drawn as large as Mermaid draws it.
    pub(super) fn drawing(self) -> Drawing {
        let texture = gdk::MemoryTexture::new(
            self.pixel_width as i32,
            self.pixel_height as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_owned(self.pixels),
            self.pixel_width as usize * 4,
        );
        Drawing::new(texture.upcast(), self.width, self.height)
    }
}

impl DiagramCard {
    /// Whether the diagram has another height in `view` than it has room
    /// for, as the view changed its width.
    pub(super) fn misfits(&self, view: &gtk::TextView) -> bool {
        self.diagram.fit(view).1.round() as i32 != self.height
    }

    /// Draws the diagram in the room above its line, in buffer coordinates.
    pub(super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        let (top, _) = self.line.line_span(view);
        if let Some(bounds) = self.diagram.bounds(view, top + PADDING as f32, visible) {
            self.diagram.snapshot(snapshot, &bounds);
        }
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
