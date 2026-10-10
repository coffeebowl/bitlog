//! Images: an image link to a file of the vault is drawn as its image below
//! its line, and the link is hidden but while the cursor is at it. Several
//! images of a line are drawn one below the other. Images are read in the
//! background and kept while the text has them, see `drawings`; links to
//! images that cannot be read are dimmed.
//!
//! The room for them is above the next line, or below the text for the last
//! line and for the one before an empty last line, which takes no tags.
//! Never below their own line: there, GTK takes a place for the end of the
//! line, counting the hidden link as shown, and aborts.

use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use bitlog_core::{Formatting, image_path};
use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, graphene};

use super::decorations::Decorations;
use super::drawings::{Drawing, Drawings, LineMark, State};
use super::styling::Styling;
use super::tags;
use crate::asset_preview::Image;

/// The space above each image of a line, and below the last.
const PADDING: i32 = 8;
/// Images are read at most as wide as the widest text, that of the note
/// view, in pixels of the view, unless the view says otherwise.
pub(super) const READ_WIDTH: i32 = 720;

/// Where the text of a view is saved, which its image links are relative
/// to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Location {
    /// Of the vault.
    pub(super) root: PathBuf,
    pub(super) file: PathBuf,
}

/// An image file as it is to be read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Request {
    path: PathBuf,
    /// So that an image is read again once it changed, or once a missing
    /// one is there.
    modified: Option<SystemTime>,
    /// The widest it is read, in pixels of the screen.
    pixels: i32,
}

/// An image as read in the background: upright, and with how large it is
/// in its file, which may be larger.
pub(super) struct ReadImage {
    image: Image,
    width: f32,
    height: f32,
}

/// An image link drawn as its image.
#[derive(Debug)]
pub(super) struct ImageCard {
    /// Of the link, in characters.
    pub(super) range: Range<i32>,
    /// The line the image is drawn below.
    line: LineMark,
    /// How far below the bottom of the line the image is drawn.
    below_line: i32,
    picture: Rc<Drawing>,
    /// How high the image is drawn.
    height: i32,
    pub(super) path: PathBuf,
}

/// Draws the image links of `formatting` to files of the vault as their
/// images, those of the text at `location`, and adds their cards to
/// `decorations`. Returns the images to read, at most `read_width` wide in
/// pixels of the view.
pub(super) fn style(
    styling: &Styling,
    formatting: &Formatting,
    decorations: &mut Decorations,
    images: &Drawings<Request>,
    location: Option<&Location>,
    read_width: i32,
) -> Vec<Request> {
    let pixels = read_width * styling.view.scale_factor();
    let mut pass = images.pass();
    // The lines with images drawn, by their start: how high the room below
    // them is so far, and the images hidden in them.
    let mut lines: Vec<(usize, i32, Vec<Range<usize>>)> = Vec::new();
    for image in formatting.images.iter().filter(|image| !image.in_table) {
        let Some(path) = location.and_then(|at| image_path(&at.root, &at.file, &image.destination))
        else {
            continue;
        };
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        let request = Request {
            path: path.clone(),
            modified,
            pixels,
        };
        let picture = match pass.state(request) {
            State::Pending => continue,
            State::Failed => {
                styling.tag(tags::BROKEN_LINK, &image.range);
                continue;
            }
            State::Done(picture) => picture,
        };
        let line_start = styling.text[..image.range.start]
            .rfind('\n')
            .map_or(0, |end| end + 1);
        if lines
            .last()
            .is_none_or(|(start, _, _)| *start != line_start)
        {
            lines.push((line_start, 0, Vec::new()));
        }
        let (_, room, hidden) = lines.last_mut().expect("just pushed");
        let height = picture.height(styling.view);
        *room += PADDING;
        decorations.images.push(ImageCard {
            range: styling.chars(&image.range),
            line: LineMark::new(&styling.buffer, styling.offset(line_start)),
            below_line: *room,
            picture,
            height,
            path,
        });
        *room += height;
        decorations.revealable.push(styling.chars(&image.range));
        if !styling.is_at(&image.range) {
            hidden.push(image.range.clone());
        }
    }
    let missing = pass.end();
    for (start, room, hidden) in lines {
        let room = room + PADDING;
        let end = styling.text[start..]
            .find('\n')
            .map_or(styling.text.len(), |end| start + end);
        if end + 1 < styling.text.len() {
            tags::make_room(styling, end + 1, room);
        } else {
            decorations.room_below_text = room;
        }
        let alone = shows_nothing_but(styling.text, start..end, &hidden);
        if alone {
            // Shown again once the cursor is in it.
            decorations.revealable.push(styling.chars(&(start..end)));
        }
        if alone && !styling.is_at(&(start..end)) {
            let line_end = (end + 1).min(styling.text.len());
            tags::hide_line(styling, start..line_end);
        } else {
            for range in &hidden {
                styling.tag(tags::HIDDEN, range);
            }
        }
    }
    missing
}

/// Whether `line` of `text` holds only whitespace besides the sorted
/// `hidden` ranges.
fn shows_nothing_but(text: &str, line: Range<usize>, hidden: &[Range<usize>]) -> bool {
    let mut start = line.start;
    for range in hidden {
        if !text[start..range.start].trim().is_empty() {
            return false;
        }
        start = range.end;
    }
    !hidden.is_empty() && text[start..line.end].trim().is_empty()
}

impl Request {
    /// Reads the image upright and at most as wide as asked, unless the
    /// file is missing or no image.
    pub(super) fn read(&self) -> Option<ReadImage> {
        let (_, width, height) = gdk_pixbuf::Pixbuf::file_info(&self.path)?;
        let pixbuf = if width > self.pixels {
            // As wide as that, as high as it takes.
            gdk_pixbuf::Pixbuf::from_file_at_scale(&self.path, self.pixels, -1, true)
        } else {
            gdk_pixbuf::Pixbuf::from_file(&self.path)
        }
        .ok()?;
        let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
        // Upright, the file's width may be the height.
        let (read_width, read_height) = (pixbuf.width() as f32, pixbuf.height() as f32);
        let enlarged = width.max(height) as f32 / read_width.max(read_height);
        Some(ReadImage {
            image: Image::new(&pixbuf)?,
            width: read_width * enlarged,
            height: read_height * enlarged,
        })
    }
}

impl ReadImage {
    /// The image, ready to be drawn as large as in its file.
    pub(super) fn drawing(self) -> Drawing {
        Drawing::new(self.image.texture(), self.width, self.height)
    }
}

impl ImageCard {
    /// Whether the image has another height in `view` than it has room
    /// for, as the view changed its width.
    pub(super) fn misfits(&self, view: &gtk::TextView) -> bool {
        self.picture.height(view) != self.height
    }

    /// Where the image is drawn in `view`, in buffer coordinates, unless
    /// it is out of the `visible` part.
    fn bounds(&self, view: &gtk::TextView, visible: &gdk::Rectangle) -> Option<graphene::Rect> {
        let (_, bottom) = self.line.line_span(view);
        self.picture
            .bounds(view, bottom + self.below_line as f32, visible)
    }

    /// Whether the image covers `x`, `y` in `view`, in buffer coordinates.
    pub(super) fn contains(&self, view: &gtk::TextView, x: i32, y: i32) -> bool {
        self.bounds(view, &view.visible_rect())
            .is_some_and(|bounds| bounds.contains_point(&graphene::Point::new(x as f32, y as f32)))
    }

    pub(super) fn snapshot(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        visible: &gdk::Rectangle,
    ) {
        if let Some(bounds) = self.bounds(view, visible) {
            self.picture.snapshot(snapshot, &bounds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_of_images_alone_show_nothing_else() {
        let text = "  ![a](a.png) ![b](b.png) ";
        let a = 2..13;
        let b = 14..25;
        assert!(shows_nothing_but(text, 0..text.len(), &[a.clone(), b]));
        assert!(!shows_nothing_but(text, 0..text.len(), &[a]));
        let after_text = 4..15;
        assert!(!shows_nothing_but("See ![a](a.png)", 0..15, &[after_text]));
        assert!(!shows_nothing_but("", 0..0, &[]));
    }
}
