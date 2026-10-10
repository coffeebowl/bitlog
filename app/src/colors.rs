//! Colours the app draws with, beside those of the theme.

use bitlog_core::{ProjectSlug, Vault};
use gtk::{gdk, glib};

/// For blocks of projects the vault does not know.
const UNKNOWN_PROJECT_COLOR: &str = "#9a9996";

/// The accent colour of the brand, used sparingly.
pub fn sea_green() -> gdk::RGBA {
    parse("#3ba99c")
}

/// `hex`, as in `#3584e4`, from the app or a project, whose colors the core
/// checks.
pub fn parse(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).expect("the colors of the app and the core are valid")
}

/// The color of the project `slug`, as in `#3584e4`, gray for one the vault
/// does not know.
pub fn project_hex<'a>(vault: &'a Vault, slug: &ProjectSlug) -> &'a str {
    vault
        .project(slug)
        .map_or(UNKNOWN_PROJECT_COLOR, |project| project.color.as_str())
}

/// The color of the project `slug`, gray for one the vault does not know.
pub fn project_color(vault: &Vault, slug: &ProjectSlug) -> gdk::RGBA {
    parse(project_hex(vault, slug))
}

/// `color` with its opacity scaled by `alpha`.
pub fn with_alpha(color: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    color.with_alpha(color.alpha() * alpha)
}

/// A dot in `color`, as in `#3584e4`, as Pango markup.
pub fn color_dot(color: &str) -> String {
    format!("<span foreground=\"{color}\">●</span>")
}

/// `text` behind a dot in `color`, as Pango markup.
pub fn dot_markup(color: &str, text: &str) -> String {
    format!("{} {}", color_dot(color), glib::markup_escape_text(text))
}

/// The colors of `parts` mixed by their weights, in the Oklab color space,
/// where colors look as between their parts, at `lightness` from 0 to 1,
/// so that light and dark colors weigh the same. As much of the color
/// as fits in sRGB at that lightness. Opaque, and transparent without
/// weight.
pub fn mix(parts: &[(gdk::RGBA, f32)], lightness: f32) -> gdk::RGBA {
    let total: f32 = parts.iter().map(|(_, weight)| weight).sum();
    if total <= 0.0 {
        return gdk::RGBA::TRANSPARENT;
    }
    let (mut a, mut b) = (0.0, 0.0);
    for (color, weight) in parts {
        let [_, part_a, part_b] = oklab(color);
        a += part_a * weight / total;
        b += part_b * weight / total;
    }
    // Less color step by step until it fits, as gray always does.
    let mut chroma = 1.0;
    let mut rgb = linear_from_oklab([lightness, a, b]);
    while chroma > 0.0 && !rgb.iter().all(|channel| (0.0..=1.0).contains(channel)) {
        chroma -= 0.02;
        rgb = linear_from_oklab([lightness, a * chroma, b * chroma]);
    }
    let [red, green, blue] = rgb.map(from_linear);
    gdk::RGBA::new(red, green, blue, 1.0)
}

/// A pastel of `color`, lighter and less colorful, in Oklab: project colors
/// look loud in large areas. Light on dark themes too, where darker shades
/// look muddy.
pub fn pastel(color: &gdk::RGBA) -> gdk::RGBA {
    let [lightness, a, b] = oklab(color);
    let lightness = lightness + (1.0 - lightness) * 0.25;
    let [red, green, blue] = linear_from_oklab([lightness, a * 0.8, b * 0.8]).map(from_linear);
    gdk::RGBA::new(red, green, blue, color.alpha())
}

/// The lightness of `color` in Oklab, from 0 to 1.
pub fn lightness(color: &gdk::RGBA) -> f32 {
    oklab(color)[0]
}

/// `color` in Oklab, see <https://bottosson.github.io/posts/oklab/>.
fn oklab(color: &gdk::RGBA) -> [f32; 3] {
    let [r, g, b] = [color.red(), color.green(), color.blue()].map(to_linear);
    let l = 0.412_221_47f32.mul_add(r, 0.536_332_55f32.mul_add(g, 0.051_445_995 * b));
    let m = 0.211_903_5f32.mul_add(r, 0.680_699_5f32.mul_add(g, 0.107_396_96 * b));
    let s = 0.088_302_46f32.mul_add(r, 0.281_718_85f32.mul_add(g, 0.629_978_7 * b));
    let [l, m, s] = [l, m, s].map(f32::cbrt);
    [
        0.210_454_26f32.mul_add(l, 0.793_617_8f32.mul_add(m, -0.004_072_047 * s)),
        1.977_998_5f32.mul_add(l, (-2.428_592_2f32).mul_add(m, 0.450_593_7 * s)),
        0.025_904_037f32.mul_add(l, 0.782_771_77f32.mul_add(m, -0.808_675_77 * s)),
    ]
}

/// The red, green and blue of `lab` in linear light, the inverse of
/// `oklab`, beyond 0 to 1 for colors sRGB lacks.
fn linear_from_oklab([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.396_337_78f32.mul_add(a, 0.215_803_76f32.mul_add(b, lightness));
    let m = (-0.105_561_346f32).mul_add(a, (-0.063_854_17f32).mul_add(b, lightness));
    let s = (-0.089_484_18f32).mul_add(a, (-1.291_485_5f32).mul_add(b, lightness));
    let [l, m, s] = [l, m, s].map(|part| part.powi(3));
    [
        4.076_741_7f32.mul_add(l, (-3.307_711_6f32).mul_add(m, 0.230_969_94 * s)),
        (-1.268_438f32).mul_add(l, 2.609_757_4f32.mul_add(m, -0.341_319_38 * s)),
        (-0.004_196_086_3f32).mul_add(l, (-0.703_418_6f32).mul_add(m, 1.707_614_7 * s)),
    ]
}

/// An sRGB channel in linear light.
fn to_linear(channel: f32) -> f32 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// A channel in linear light in sRGB, clamped to what it can show.
fn from_linear(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055f32.mul_add(channel.powf(1.0 / 2.4), -0.055)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(color: &gdk::RGBA, other: &gdk::RGBA) -> bool {
        [
            color.red() - other.red(),
            color.green() - other.green(),
            color.blue() - other.blue(),
        ]
        .iter()
        .all(|diff| diff.abs() < 0.002)
    }

    #[test]
    fn mixes() {
        let blue = gdk::RGBA::parse("#3584e4").unwrap();
        let red = gdk::RGBA::parse("#e01b24").unwrap();
        let yellow = gdk::RGBA::parse("#f6d32d").unwrap();
        let at = lightness(&blue);
        assert!(close(&mix(&[(blue, 3.0)], at), &blue));
        assert!(close(&mix(&[(blue, 1.0), (red, 0.0)], at), &blue));
        let half = mix(&[(blue, 1.0), (red, 1.0)], at);
        assert!(half.red() > blue.red() && half.red() < red.red());
        // Yellow as dark as blue, still yellow.
        let dark = mix(&[(yellow, 1.0)], at);
        assert!((lightness(&dark) - at).abs() < 0.01);
        assert!(dark.red() > dark.blue() && dark.green() > dark.blue());
        assert_eq!(mix(&[], at), gdk::RGBA::TRANSPARENT);
    }

    #[test]
    fn pastels_are_lighter_and_keep_their_hue() {
        let blue = gdk::RGBA::parse("#3584e4").unwrap();
        let soft = pastel(&blue);
        assert!(lightness(&soft) > lightness(&blue));
        assert!(soft.blue() > soft.green() && soft.green() > soft.red());
        let white = gdk::RGBA::parse("#ffffff").unwrap();
        assert!(close(&pastel(&white), &white));
    }
}
