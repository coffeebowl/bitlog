//! The page an asset shows in the grid: the start of its content where it
//! can be read quickly, such as an image, the first page of a PDF, a text
//! file or the thumbnail a LibreOffice document holds, and an icon for its
//! type otherwise.

use std::fs;
use std::io::Read;
use std::path::Path;

use bitlog_core::{Asset, Vault};
use gtk::{cairo, gdk, gdk_pixbuf, gio, glib};
use sourceview5::prelude::*;

use crate::markdown_view::{Highlighting, Themes, themes};
use crate::miniature::Miniature;

/// The size of the icon on the page of an asset.
const ICON_SIZE: i32 = 48;

/// The size an image is read at for its page, enough to cover it.
const IMAGE_SIZE: i32 = 720;

/// The largest image, in bytes, that its page shows. Larger ones show the
/// icon of their type, as reading them would take too long.
const MAX_IMAGE_SIZE: u64 = 50_000_000;

/// The bytes read of a file to tell whether it is text and show its start.
const TEXT_BYTES: u64 = 16 * 1024;

/// The lines of a text file its page shows at most, a few more than fit.
const TEXT_LINES: usize = 24;

/// The characters of a line its page shows at most, as much as fits on two
/// rows. Laying out more, such as a file of one long line, would slow down
/// every move to and from the project.
const LINE_CHARS: usize = 80;

/// Where LibreOffice keeps the thumbnail in its documents.
const ODF_THUMBNAIL: &str = "Thumbnails/thumbnail.png";

/// The square page of `asset` in `vault`. It shows the icon of the asset's
/// type at first, and its content once read in the background, if it can.
pub fn asset_page(vault: &Vault, asset: &Asset) -> gtk::Overlay {
    let name = asset.path.name().to_owned();
    let (content_type, _) = gio::content_type_guess(Some(&name), None);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // Above the page, not on it, so that it is not scaled down with it.
    let icon = gtk::Image::builder()
        .gicon(&gio::content_type_get_symbolic_icon(&content_type))
        .pixel_size(ICON_SIZE)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let page = gtk::Overlay::builder()
        .child(&Miniature::square(&content))
        .overflow(gtk::Overflow::Hidden)
        .build();
    page.add_overlay(&icon);

    let path = vault.asset_path(&asset.path);
    let mime_type = gio::content_type_get_mime_type(&content_type).unwrap_or_default();
    if mime_type.starts_with("image/") {
        if asset.size <= MAX_IMAGE_SIZE {
            show_preview(&content, &icon, move || {
                read_image(&path).map(Preview::Image)
            });
        }
    } else if mime_type == "application/pdf" {
        show_preview(&content, &icon, move || {
            read_pdf_page(&path).map(Preview::Image)
        });
    } else if mime_type.starts_with("application/vnd.oasis.opendocument.") {
        show_preview(&content, &icon, move || {
            read_odf_thumbnail(&path).map(Preview::Image)
        });
    } else {
        let themes = themes();
        show_preview(&content, &icon, move || read_text(&path, &name, &themes));
    }
    page
}

/// What the page of an asset shows instead of the icon.
enum Preview {
    Image(Image),
    /// Highlighted if the language is known.
    Text {
        text: String,
        highlighting: Option<Highlighting>,
    },
}

/// An image as read, to be shown in the main thread, which alone may hold
/// a pixbuf or texture.
pub(crate) struct Image {
    width: i32,
    height: i32,
    format: gdk::MemoryFormat,
    pixels: glib::Bytes,
    /// The bytes from one row to the next.
    stride: usize,
}

impl Image {
    pub(crate) fn new(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<Self> {
        Some(Self {
            width: pixbuf.width(),
            height: pixbuf.height(),
            format: if pixbuf.has_alpha() {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            },
            pixels: pixbuf.read_pixel_bytes(),
            stride: usize::try_from(pixbuf.rowstride()).ok()?,
        })
    }

    pub(crate) fn texture(&self) -> gdk::Texture {
        gdk::MemoryTexture::new(
            self.width,
            self.height,
            self.format,
            &self.pixels,
            self.stride,
        )
        .upcast()
    }
}

/// Runs `read` in the background, then shows what it read in `content`
/// instead of `icon`. If it reads nothing, the icon stays.
fn show_preview(
    content: &gtk::Box,
    icon: &gtk::Image,
    read: impl FnOnce() -> Option<Preview> + Send + 'static,
) {
    glib::spawn_future_local(glib::clone!(
        #[weak]
        content,
        #[weak]
        icon,
        async move {
            let preview = gio::spawn_blocking(read)
                .await
                .expect("reading a preview does not panic");
            let widget = match preview {
                Some(Preview::Image(image)) => picture(&image).upcast::<gtk::Widget>(),
                Some(Preview::Text { text, highlighting }) => {
                    text_label(text, highlighting).upcast()
                }
                None => return,
            };
            icon.set_visible(false);
            content.append(&widget);
        }
    ));
}

/// `image`, covering the page.
fn picture(image: &Image) -> gtk::Picture {
    gtk::Picture::builder()
        .paintable(&image.texture())
        .content_fit(gtk::ContentFit::Cover)
        .vexpand(true)
        .build()
}

/// `text` in a monospace font, with `highlighting` in the colours of the
/// light or dark style.
///
/// A label, as it is far quicker to lay out than a text view, which would
/// slow down every move to and from the project.
fn text_label(text: String, highlighting: Option<Highlighting>) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(&text)
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .margin_top(40)
        .margin_start(40)
        .margin_end(40)
        .css_classes(["monospace"])
        .build();
    if let Some(highlighting) = highlighting {
        adw::StyleManager::default()
            .bind_property("dark", &label, "attributes")
            .transform_to(move |_, dark: bool| Some(highlighting.attributes(&text, dark)))
            .sync_create()
            .build();
    }
    label
}

/// The image at `path`, upright and at most as large as a page, or `None`
/// if it cannot be read.
fn read_image(path: &Path) -> Option<Image> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, IMAGE_SIZE, IMAGE_SIZE, true).ok()?;
    Image::new(&pixbuf.apply_embedded_orientation().unwrap_or(pixbuf))
}

/// The first page of the PDF at `path`, as wide as an image is read, or
/// `None` if it cannot be read, as when it is protected by a password.
fn read_pdf_page(path: &Path) -> Option<Image> {
    let file = gio::File::for_path(path);
    let document = poppler::Document::from_gfile(&file, None, gio::Cancellable::NONE).ok()?;
    let page = document.page(0)?;
    let (width, height) = page.size();
    let scale = f64::from(IMAGE_SIZE) / width.max(height);
    let surface = cairo::ImageSurface::create(
        cairo::Format::Rgb24,
        (width * scale).ceil() as i32,
        (height * scale).ceil() as i32,
    )
    .ok()?;
    {
        let context = cairo::Context::new(&surface).ok()?;
        // Pages are drawn on white paper, which PDFs leave out.
        context.set_source_rgb(1.0, 1.0, 1.0);
        context.paint().ok()?;
        context.scale(scale, scale);
        page.render(&context);
    }
    surface.flush();
    let (width, height, stride) = (surface.width(), surface.height(), surface.stride());
    let pixels = glib::Bytes::from_owned(surface.take_data().ok()?.to_vec());
    Some(Image {
        width,
        height,
        // Cairo keeps a pixel as one native-endian 32-bit number.
        format: if cfg!(target_endian = "little") {
            gdk::MemoryFormat::B8g8r8x8
        } else {
            gdk::MemoryFormat::X8r8g8b8
        },
        pixels,
        stride: usize::try_from(stride).ok()?,
    })
}

/// The thumbnail that LibreOffice puts into its documents, an image of
/// the first page, or `None` if the document at `path` has none.
fn read_odf_thumbnail(path: &Path) -> Option<Image> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
    let mut png = Vec::new();
    archive
        .by_name(ODF_THUMBNAIL)
        .ok()?
        .read_to_end(&mut png)
        .ok()?;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(png));
    Image::new(&gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE).ok()?)
}

/// The first lines of the file `name` at `path`, if it is text that is
/// not empty, highlighted with `themes` as the extension of the name tells.
fn read_text(path: &Path, name: &str, themes: &Themes) -> Option<Preview> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(TEXT_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let (content_type, _) = gio::content_type_guess(Some(name), Some(bytes.as_slice()));
    if !gio::content_type_is_a(&content_type, "text/plain") || bytes.contains(&0) {
        return None;
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        // Reading stopped within a character.
        Err(err) if err.error_len().is_none() => {
            std::str::from_utf8(&bytes[..err.valid_up_to()]).expect("valid up to there")
        }
        Err(_) => return None,
    };
    let text: Vec<String> = text
        .lines()
        .take(TEXT_LINES)
        .map(|line| line.chars().take(LINE_CHARS).collect())
        .collect();
    let text = text.join("\n");
    if text.trim().is_empty() {
        return None;
    }
    let language = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or(name);
    Some(Preview::Text {
        highlighting: themes.highlight(language, &text),
        text,
    })
}
