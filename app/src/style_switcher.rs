//! The choice between the system style, light and dark, as three round
//! buttons at the top of the main menu, like in GNOME Text Editor.

use adw::prelude::*;
use gettextrs::gettext;
use gtk::gio;

/// The menu item the switcher takes the place of.
const MENU_ITEM: &str = "style-switcher";

/// Keeps the app in the color scheme the settings hold and adds
/// `app.color-scheme`, which changes it.
pub fn follow_settings(app: &adw::Application, settings: &gio::Settings) {
    app.add_action(&settings.create_action("color-scheme"));
    apply(settings);
    settings.connect_changed(Some("color-scheme"), |settings, _| apply(settings));
}

fn apply(settings: &gio::Settings) {
    let scheme = match settings.string("color-scheme").as_str() {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

/// Puts a switcher into the menu `button` shows.
pub fn add_to(button: &gtk::MenuButton) {
    let popover = button
        .popover()
        .and_downcast::<gtk::PopoverMenu>()
        .expect("menu buttons with a menu model show a popover menu");
    popover.add_child(&switcher(), MENU_ITEM);
}

fn switcher() -> gtk::Box {
    let switcher = gtk::Box::builder()
        .spacing(18)
        .halign(gtk::Align::Center)
        .css_classes(["style-switcher"])
        .build();
    let choices = [
        ("default", "follow", gettext("Follow System Style")),
        ("light", "light", gettext("Light Style")),
        ("dark", "dark", gettext("Dark Style")),
    ];
    let mut group: Option<gtk::CheckButton> = None;
    for (scheme, class, label) in choices {
        let button = gtk::CheckButton::builder()
            .action_name("app.color-scheme")
            .action_target(&scheme.to_variant())
            .tooltip_text(&label)
            .css_classes([class])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        button.set_group(group.as_ref());
        group.get_or_insert(button.clone());
        switcher.append(&button);
    }
    switcher
}
