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
    // JPEG has no transparency, so it goes onto white, into a picture
    // without an alpha channel: the JPEG writer refuses one.
    let small = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, size, size)
        .ok_or_else(|| {
            glib::Error::new(gdk_pixbuf::PixbufError::InsufficientMemory, "cannot scale")
        })?;
    small.fill(0xffffffff);
    let scale = f64::from(size) / f64::from(side);
    square.composite(
        &small,
        0,
        0,
        size,
        size,
        0.0,
        0.0,
        scale,
        scale,
        gdk_pixbuf::InterpType::Hyper,
        255,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pictures_become_jpeg() {
        let png = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 40, 30)
            .unwrap()
            .save_to_bufferv("png", &[])
            .unwrap();
        let jpeg = jpeg(Source::Png(glib::Bytes::from_owned(png))).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]));
        let read = gdk_pixbuf::Pixbuf::from_stream(
            &gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(jpeg)),
            gio::Cancellable::NONE,
        )
        .unwrap();
        assert_eq!((read.width(), read.height()), (30, 30));
    }
}
