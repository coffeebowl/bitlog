//! The context menu of editable texts: GTK's entries, then formatting with
//! its shortcuts, so they can be found and learned, and the help on Markdown.

use gettextrs::gettext;
use gtk::gio;
use gtk::prelude::*;

use super::MarkdownView;

impl MarkdownView {
    /// Adds the entries while the text is editable, as they change it.
    pub(super) fn add_menu(&self) {
        let menu = menu();
        self.connect_editable_notify(move |view| {
            view.set_extra_menu(view.is_editable().then_some(&menu));
        });
    }
}

fn menu() -> gio::Menu {
    let formatting = gio::Menu::new();
    for (label, action, accel) in [
        (gettext("_Bold"), "markdown.bold", "<Control>b"),
        (gettext("_Italic"), "markdown.italic", "<Control>i"),
        (gettext("C_ode"), "markdown.code", "<Control>e"),
        (
            gettext("Toggle Tas_k"),
            "markdown.toggle-task",
            "<Control>l",
        ),
    ] {
        let item = gio::MenuItem::new(Some(&label), Some(action));
        item.set_attribute_value("accel", Some(&accel.to_variant()));
        formatting.append_item(&item);
    }
    let help = gio::Menu::new();
    help.append(Some(&gettext("_Markdown Help")), Some("markdown.help"));
    let menu = gio::Menu::new();
    menu.append_section(None, &formatting);
    menu.append_section(None, &help);
    menu
}
