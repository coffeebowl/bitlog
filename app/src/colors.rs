//! Colours the app draws with, beside those of the theme.

use gtk::gdk;

/// For blocks of projects the vault does not know.
pub const UNKNOWN_PROJECT_COLOR: &str = "#9a9996";

/// The accent colour of the brand, used sparingly.
pub fn sea_green() -> gdk::RGBA {
    gdk::RGBA::parse("#3ba99c").expect("the colour is valid")
}

/// `color` with its opacity scaled by `alpha`.
pub fn with_alpha(color: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    color.with_alpha(color.alpha() * alpha)
}

/// A dot in `color`, as in `#3584e4`, as Pango markup.
pub fn color_dot(color: &str) -> String {
    format!("<span foreground=\"{color}\">●</span>")
}
