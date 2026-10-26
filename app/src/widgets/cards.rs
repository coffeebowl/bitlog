//! Cards of notes, assets and their folders in the grids of the project
//! view, the assets view and the notes page, with the miniatures of their
//! pages.

use adw::prelude::*;
use bitlog_core::{Asset, AssetPath, NotePath, Vault, without_front_matter};
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::markdown_view::MarkdownView;
use crate::widgets::asset_preview::asset_page;
use crate::widgets::miniature::Miniature;

/// The lines of a note its miniature formats at most, more than fit.
const PREVIEW_LINES: usize = 50;
/// How wide the miniature of a note reads its images at most, in pixels of
/// its page, which is shown far smaller.
const PREVIEW_IMAGE_WIDTH: i32 = 270;
/// The size of the folder on the card of a folder.
const FOLDER_SIZE: i32 = 96;

/// The text of `note`, or none if it cannot be read.
fn note_text(vault: &Vault, note: &NotePath) -> String {
    vault.load_note(note).map_or_else(
        |err| {
            glib::g_warning!("bitlog", "{err}");
            String::new()
        },
        |file| file.text,
    )
}

/// Shows the start of a note's text, its Markdown formatted, laid out as on
/// a page.
pub fn note_preview() -> MarkdownView {
    let preview: MarkdownView = glib::Object::new();
    preview.set_full(true);
    preview.set_top_margin(48);
    preview.set_bottom_margin(48);
    preview.set_left_margin(56);
    preview.set_right_margin(56);
    // Clicks go to the card, which opens the note.
    preview.set_can_target(false);
    preview.set_focusable(false);
    preview.set_image_width(PREVIEW_IMAGE_WIDTH);
    preview
}

/// Shows the start of the text of `note` in `preview`, its images
/// included, unless it shows it already.
pub fn show_preview(preview: &MarkdownView, vault: &Vault, note: &NotePath) {
    preview.set_location(vault.root(), &vault.note_path(note));
    let text = note_text(vault, note);
    let text = preview_text(without_front_matter(&text).trim_start_matches(['\r', '\n']));
    if !preview.shows(text) {
        preview.set_markdown(text);
    }
}

/// A note in the grid: `preview` as the miniature of a page, above its
/// name, `details` if given, and menu. The grid opens it when activated.
pub fn note_card(
    note: &NotePath,
    preview: &MarkdownView,
    details: Option<&gtk::Label>,
) -> gtk::FlowBoxChild {
    let page = Miniature::new(preview);
    page.add_css_class("miniature-page");
    page_card(
        &page,
        note.name(),
        note.name(),
        details,
        &gettext("Note Menu"),
        &note_menu(note),
    )
}

/// A card in a grid of notes or assets: `page`, the miniature of a page or
/// a folder, above `name`, with `tooltip`, `details` if given, and `menu`.
fn page_card(
    page: &impl IsA<gtk::Widget>,
    name: &str,
    tooltip: &str,
    details: Option<&gtk::Label>,
    menu_tooltip: &str,
    menu: &gio::Menu,
) -> gtk::FlowBoxChild {
    // Takes up rounding, so that the names line up.
    page.set_vexpand(true);

    let labels = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    labels.append(
        &gtk::Label::builder()
            .label(name)
            .tooltip_text(tooltip)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            // Leaves the width of the card to the page.
            .max_width_chars(1)
            .margin_start(3)
            .build(),
    );
    if let Some(details) = details {
        details.set_xalign(0.0);
        details.set_ellipsize(gtk::pango::EllipsizeMode::End);
        details.set_max_width_chars(1);
        details.set_margin_start(3);
        details.add_css_class("caption");
        details.add_css_class("dim-label");
        labels.append(details);
    }
    let footer = gtk::Box::builder().spacing(3).build();
    footer.append(&labels);
    footer.append(
        &gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(menu_tooltip)
            .menu_model(menu)
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build(),
    );

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    content.append(page);
    content.append(&footer);
    gtk::FlowBoxChild::builder()
        .child(&content)
        .width_request(120)
        .css_classes(["page-card"])
        .build()
}

/// The start of `text`, as much as a preview formats.
fn preview_text(text: &str) -> &str {
    let end = text
        .match_indices('\n')
        .nth(PREVIEW_LINES - 1)
        .map_or(text.len(), |(end, _)| end);
    &text[..end]
}

/// Opening the note `note` in another app, showing it in its folder,
/// renaming it and moving it to the trash.
pub fn note_menu(note: &NotePath) -> gio::Menu {
    let target = note.to_string().to_variant();
    let menu = gio::Menu::new();
    for (label, action) in [
        (gettext("_Open Externally"), "note.open-file"),
        (gettext("_Show in Folder"), "note.show-file"),
        (gettext("_Rename…"), "note.rename"),
        (gettext("Move to _Trash"), "note.trash"),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target));
        menu.append_item(&item);
    }
    menu
}

/// An asset in the grid: its page, above its name, size and menu. The grid
/// opens it when activated.
pub fn asset_card(vault: &Vault, asset: &Asset) -> gtk::FlowBoxChild {
    let path = &asset.path;
    let page = asset_page(vault, asset);
    page.add_css_class("miniature-page");
    page_card(
        &page,
        path.name(),
        path.name(),
        Some(&gtk::Label::new(Some(&glib::format_size(asset.size)))),
        &gettext("File Menu"),
        &asset_menu(path),
    )
}

/// A folder of assets in the grid: a folder in place of a page, above its
/// name and menu. The grid opens it when activated.
pub fn folder_card(folder: &AssetPath) -> gtk::FlowBoxChild {
    let page = gtk::Overlay::builder()
        .child(&Miniature::square(&gtk::Box::default()))
        .build();
    page.add_overlay(
        &gtk::Image::builder()
            .icon_name("asset-folder-symbolic")
            .pixel_size(FOLDER_SIZE)
            .css_classes(["dim-label"])
            .build(),
    );
    page_card(
        &page,
        folder.name(),
        folder.name(),
        None,
        &gettext("Folder Menu"),
        &asset_menu(folder),
    )
}

/// Showing the file or folder `asset` in its folder, renaming it and moving
/// it to the trash. Opening it is what clicking its card does.
fn asset_menu(asset: &AssetPath) -> gio::Menu {
    let target = asset.to_string().to_variant();
    let menu = gio::Menu::new();
    for (label, action) in [
        (gettext("_Show in Folder"), "assets.show"),
        (gettext("_Rename…"), "assets.rename"),
        (gettext("Move to _Trash"), "assets.trash"),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target));
        menu.append_item(&item);
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_start_of_long_texts() {
        assert_eq!(preview_text("# Notes\n\nShort."), "# Notes\n\nShort.");
        let long: Vec<String> = (1..=80).map(|line| line.to_string()).collect();
        let preview = preview_text(&long.join("\n")).to_owned();
        assert_eq!(preview.lines().count(), PREVIEW_LINES);
        assert!(preview.ends_with("\n50"));
    }
}
