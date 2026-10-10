//! The context menu of editable texts: GTK's entries, then formatting with
//! its shortcuts, so they can be found and learned, and the help on Markdown.

use gettextrs::gettext;
use gtk::prelude::*;
use gtk::{gdk, gio};

use super::{EDIT_ACTIONS, MarkdownView};

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
    let labels = [
        gettext("_Bold"),
        gettext("_Italic"),
        gettext("C_ode"),
        gettext("Toggle Tas_k"),
    ];
    for (label, (action, key)) in labels.iter().zip(EDIT_ACTIONS) {
        let item = gio::MenuItem::new(Some(label), Some(action));
        let accel = gtk::accelerator_name(key, gdk::ModifierType::CONTROL_MASK);
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
