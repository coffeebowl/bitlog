//! The card that a click on a mention like `@kim` shows: the person's
//! picture and name, their role in the project of the text, how to reach them, and a button
//! to open the contact.

use std::ops::Range;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, ContactSlug};
use gettextrs::gettext;
use gtk::{gdk, glib};

use super::MarkdownView;
use crate::contact_image::avatar;
use crate::contact_view::{mail_uri, phone_uri};
use crate::format::contact_label;
use crate::launch;

const CARD_WIDTH: i32 = 280;
const AVATAR_SIZE: i32 = 48;

impl MarkdownView {
    /// Shows the card of the contact `slug`, pointing to its mention at the
    /// character range `range`.
    pub(super) fn show_contact_card(&self, slug: &ContactSlug, range: &Range<i32>) {
        let imp = self.imp();
        let card = {
            let contacts = imp.contacts.borrow();
            let Some(contacts) = contacts.as_ref() else {
                return;
            };
            let Some(contact) = contacts.contacts.iter().find(|c| c.slug == *slug) else {
                return;
            };
            let role = contacts
                .project
                .as_ref()
                .and_then(|project| contact.role(project));
            let avatar = match &*imp.location.borrow() {
                Some(location) => avatar(&location.root, contact, AVATAR_SIZE),
                None => adw::Avatar::new(AVATAR_SIZE, Some(&contact.name), true),
            };
            card(contact, role, &avatar)
        };
        card.set_pointing_to(Some(&self.text_rectangle(range)));
        card.set_parent(self);
        card.connect_closed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |card| {
                // Not while GTK still hides it.
                let card = card.clone();
                glib::idle_add_local_once(move || card.unparent());
                view.imp().contact_card.take();
            }
        ));
        imp.contact_card.replace(Some(card.clone()));
        card.popup();
    }

    /// Where the text at the character range `range` is shown, in widget
    /// coordinates, or its start if it spans more than one line. Between
    /// the cursor positions, which leave out hidden text.
    fn text_rectangle(&self, range: &Range<i32>) -> gdk::Rectangle {
        let buffer = self.buffer();
        let (start, _) = self.cursor_locations(Some(&buffer.iter_at_offset(range.start)));
        let (end, _) = self.cursor_locations(Some(&buffer.iter_at_offset(range.end)));
        let width = if end.y() == start.y() {
            end.x() - start.x()
        } else {
            1
        };
        let (x, y) =
            self.buffer_to_window_coords(gtk::TextWindowType::Widget, start.x(), start.y());
        gdk::Rectangle::new(x, y, width, start.height())
    }
}

/// A card of `contact` with their `avatar` and their `role` in the project
/// of the text.
fn card(contact: &Contact, role: Option<&str>, avatar: &adw::Avatar) -> gtk::Popover {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .width_request(CARD_WIDTH)
        .build();
    let heading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .valign(gtk::Align::Center)
        .build();
    heading.append(
        &gtk::Label::builder()
            .label(&contact.name)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["title-4"])
            .build(),
    );
    if let Some(role) = role.filter(|role| !role.is_empty()) {
        heading.append(
            &gtk::Label::builder()
                .label(role)
                .wrap(true)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    let person = gtk::Box::builder().spacing(12).build();
    person.append(avatar);
    person.append(&heading);
    content.append(&person);

    let ways = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    // The preferred ones.
    if let Some((label, address)) = contact.emails.first() {
        ways.append(&way_row(address, label, mail_uri(address)));
    }
    if let Some((label, number)) = contact.phones.first() {
        ways.append(&way_row(number, label, phone_uri(number)));
    }
    if ways.first_child().is_some() {
        content.append(&ways);
    }

    let open = gtk::Button::builder()
        .label(gettext("_Open Contact"))
        .use_underline(true)
        .css_classes(["pill"])
        .halign(gtk::Align::Center)
        .build();
    let slug = contact.slug.to_string();
    content.append(&open);
    let card = gtk::Popover::builder().child(&content).build();
    open.connect_clicked(glib::clone!(
        #[weak]
        card,
        move |open| {
            open.activate_action("win.show-contact", Some(&slug.to_variant()))
                .expect("the window shows contacts");
            card.popdown();
        }
    ));
    card
}

/// A row with `text` and its `label`, which opens `uri` when activated.
fn way_row(text: &str, label: &str, uri: String) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(text)
        .subtitle(contact_label(label))
        .use_markup(false)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    row.connect_activated(move |row| {
        let (row, uri) = (row.clone(), uri.clone());
        glib::spawn_future_local(async move { launch::open_uri(&row, &uri).await });
    });
    row
}
