//! The rows of the dialog that hold texts by their labels: phone numbers,
//! e-mail addresses, other fields and roles, and the menus that add them.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{EMAIL_LABELS, PHONE_LABELS};
use gettextrs::gettext;
use gtk::{gio, glib};

use super::ContactDialog;
use crate::dialogs::note_dialogs::ask_name;
use crate::format::contact_label;

/// The kinds of fields that hold texts by their labels, any number each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Phone,
    Email,
    Field,
    /// Labeled by the slug of the project.
    Role,
}

/// A row of a text with its label.
#[derive(Debug)]
pub struct Labeled {
    kind: Kind,
    label: String,
    row: adw::EntryRow,
}

impl ContactDialog {
    /// The labels and texts of the rows of `kind` in their order, those
    /// without a text left out, apart from roles, which may be empty.
    pub(super) fn texts(&self, kind: Kind) -> Vec<(String, String)> {
        self.imp()
            .labeled
            .borrow()
            .iter()
            .filter(|labeled| labeled.kind == kind)
            .map(|labeled| (labeled.label.clone(), labeled.row.text().trim().to_owned()))
            .filter(|(_, text)| kind == Kind::Role || !text.is_empty())
            .collect()
    }

    pub(super) fn has_kind(&self, kind: Kind) -> bool {
        self.imp()
            .labeled
            .borrow()
            .iter()
            .any(|labeled| labeled.kind == kind)
    }

    /// Whether there is a row of `kind` labeled `label`.
    pub(super) fn has_label(&self, kind: Kind, label: &str) -> bool {
        self.imp()
            .labeled
            .borrow()
            .iter()
            .any(|labeled| labeled.kind == kind && labeled.label == label)
    }

    /// Adds a row of `kind` labeled `label`, asking for the label if it is
    /// empty, and lets the user type its text.
    pub(super) async fn add_labeled(&self, kind: Kind, label: String) {
        let label = if label.is_empty() {
            let heading = match kind {
                Kind::Phone => gettext("Label for Phone Number"),
                Kind::Email => gettext("Label for Email Address"),
                Kind::Field | Kind::Role => gettext("Name of Field"),
            };
            let dialog = self.clone();
            let label = ask_name(self, &heading, &gettext("_Add"), "", move |label| {
                !label.is_empty() && !dialog.has_label(kind, label)
            })
            .await;
            match label {
                Some(label) => label,
                None => return,
            }
        } else {
            label
        };
        let row = self.add_row(kind, &label, "");
        self.update_menus();
        row.grab_focus();
    }

    /// Adds a row of `kind` with `text` labeled `label`, after the others
    /// of its kind.
    pub(super) fn add_row(&self, kind: Kind, label: &str, text: &str) -> adw::EntryRow {
        let imp = self.imp();
        let title = match kind {
            Kind::Phone | Kind::Email => contact_label(label),
            Kind::Field => label.to_owned(),
            Kind::Role => imp
                .projects
                .borrow()
                .iter()
                .find(|(slug, _)| slug.as_str() == label)
                .map_or_else(|| label.to_owned(), |(_, name)| name.clone()),
        };
        let row = adw::EntryRow::builder()
            .title(title)
            .text(text)
            .activates_default(true)
            .build();
        let group = match kind {
            Kind::Phone => &imp.phone_group,
            Kind::Email => &imp.email_group,
            Kind::Field => &imp.details_group,
            Kind::Role => &imp.projects_group,
        };
        row.add_suffix(&remove_button(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            #[weak]
            group,
            #[weak]
            row,
            move || {
                group.remove(&row);
                dialog
                    .imp()
                    .labeled
                    .borrow_mut()
                    .retain(|labeled| labeled.row != row);
                dialog.update_menus();
            }
        )));
        group.add(&row);
        imp.labeled.borrow_mut().push(Labeled {
            kind,
            label: label.to_owned(),
            row: row.clone(),
        });
        row
    }

    /// Offers to add what the contact lacks, in the menus of the buttons
    /// that add numbers, addresses, projects and fields.
    pub(super) fn update_menus(&self) {
        let imp = self.imp();
        let menu = |items: Vec<(String, &str, String)>| {
            let menu = gio::Menu::new();
            for (label, action, target) in items {
                // Underscores would mark a mnemonic.
                let item = gio::MenuItem::new(Some(&label.replace('_', "__")), None);
                item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
                menu.append_item(&item);
            }
            menu
        };
        let labels = |kind: Kind, action: &'static str, labels: &[&str]| {
            let mut items: Vec<(String, &str, String)> = labels
                .iter()
                .filter(|label| !self.has_label(kind, label))
                .map(|label| (contact_label(label), action, (*label).to_owned()))
                .collect();
            items.push((gettext("Other…"), action, String::new()));
            menu(items)
        };
        imp.phone_button.set_menu_model(Some(&labels(
            Kind::Phone,
            "contact.add-phone",
            &PHONE_LABELS,
        )));
        imp.email_button.set_menu_model(Some(&labels(
            Kind::Email,
            "contact.add-email",
            &EMAIL_LABELS,
        )));

        let projects: Vec<(String, &str, String)> = imp
            .projects
            .borrow()
            .iter()
            .filter(|(slug, _)| !self.has_label(Kind::Role, slug.as_str()))
            .map(|(slug, name)| (name.clone(), "contact.add-role", slug.to_string()))
            .collect();
        imp.projects_button.set_sensitive(!projects.is_empty());
        imp.projects_button.set_menu_model(Some(&menu(projects)));

        let mut fields = Vec::new();
        for (row, label, field) in [
            (
                imp.company_row.upcast_ref::<gtk::Widget>(),
                gettext("Company"),
                "company",
            ),
            (
                imp.hours_row.upcast_ref(),
                gettext("Working Hours"),
                "hours",
            ),
            (
                imp.timezone_row.upcast_ref(),
                gettext("Time Zone"),
                "timezone",
            ),
            (
                imp.birthday_row.upcast_ref(),
                gettext("Birthday"),
                "birthday",
            ),
            (imp.note_group.upcast_ref(), gettext("Note"), "note"),
        ] {
            let offered = field != "timezone" || imp.has_timezones.get();
            if !row.is_visible() && offered {
                fields.push((label, "contact.show-field", field.to_owned()));
            }
        }
        // Hidden fields leave a group of no rows, which would still show.
        let details = [
            imp.company_row.upcast_ref::<gtk::Widget>(),
            imp.hours_row.upcast_ref(),
            imp.timezone_row.upcast_ref(),
            imp.birthday_row.upcast_ref(),
        ];
        let has_details = details.iter().any(|row| row.is_visible()) || self.has_kind(Kind::Field);
        imp.details_group.set_visible(has_details);
        let fixed = menu(fields);
        // Suggested in the language of the app; once added, the label is
        // the user's text.
        let suggested = [
            gettext("Office"),
            "GitHub".to_owned(),
            "LinkedIn".to_owned(),
            gettext("Website"),
        ];
        let mut others: Vec<(String, &str, String)> = suggested
            .into_iter()
            .filter(|label| !self.has_label(Kind::Field, label))
            .map(|label| (label.clone(), "contact.add-field", label))
            .collect();
        others.push((gettext("Other…"), "contact.add-field", String::new()));
        let all = gio::Menu::new();
        all.append_section(None, &fixed);
        all.append_section(None, &menu(others));
        imp.add_field_button.set_menu_model(Some(&all));
    }
}

/// A button at the end of a row that runs `remove`.
pub(super) fn remove_button(remove: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("list-remove-symbolic")
        .tooltip_text(gettext("Remove"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.connect_clicked(move |_| remove());
    button
}
