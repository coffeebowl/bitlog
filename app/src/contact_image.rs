//! The pictures of contacts: made small for the vault, and shown in avatars.

use std::path::{Path, PathBuf};

use bitlog_core::{Contact, ContactSlug};
use gtk::{gdk, gdk_pixbuf, gio, glib};

/// Pictures are square, this many pixels wide: enough for the largest
/// avatar at twice its size.
const SIZE: i32 = 256;

/// The JPEG quality, small enough for vaults kept in Git.
const QUALITY: &str = "80";

/// What a picture is made from.
pub enum Source {
    File(PathBuf),
    /// An image as PNG, as from the clipboard.
    Png(glib::Bytes),
}

/// The picture of `source` as JPEG, cut square from its middle and made
/// small. Takes a while for large photos.
pub fn jpeg(source: Source) -> Result<Vec<u8>, glib::Error> {
    let pixbuf = match source {
        Source::File(path) => gdk_pixbuf::Pixbuf::from_file(path)?,
        Source::Png(bytes) => gdk_pixbuf::Pixbuf::from_stream(
            &gio::MemoryInputStream::from_bytes(&bytes),
            gio::Cancellable::NONE,
        )?,
    };
    // Photos taken upright may be stored on their side.
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let side = pixbuf.width().min(pixbuf.height());
    let square = pixbuf.new_subpixbuf(
        (pixbuf.width() - side) / 2,
        (pixbuf.height() - side) / 2,
        side,
        side,
    );
    let size = side.min(SIZE);
    let interpolation = gdk_pixbuf::InterpType::Hyper;
    // JPEG has no transparency, so it goes onto white.
    let small = if square.has_alpha() {
        square.composite_color_simple(size, size, interpolation, 255, 16, 0xffffff, 0xffffff)
    } else {
        square.scale_simple(size, size, interpolation)
    };
    let small = small.ok_or_else(|| {
        glib::Error::new(gdk_pixbuf::PixbufError::InsufficientMemory, "cannot scale")
    })?;
    small.save_to_bufferv("jpeg", &[("quality", QUALITY)])
}

/// The picture of the contact `slug` in the vault at `root`, if it has one
/// that can be read.
pub fn picture(root: &Path, slug: &ContactSlug) -> Option<gdk::Texture> {
    gdk::Texture::from_filename(Contact::image_path(root, slug)).ok()
}

/// An avatar of `contact` of the vault at `root`, `size` pixels wide, with
/// their picture or else their initials.
pub fn avatar(root: &Path, contact: &Contact, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(&contact.name), true);
    avatar.set_custom_image(picture(root, &contact.slug).as_ref());
    avatar
}
