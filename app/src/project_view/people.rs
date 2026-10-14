//! What the project view does with the people working on the project.

use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, ContactSlug, ProjectSlug, SaveError, Vault};
use gettextrs::gettext;
use gtk::{gio, glib};

use super::ProjectView;
use crate::alert::show_error;
use crate::contact_dialog::ContactDialog;
use crate::contact_image::avatar;
use crate::note_dialogs::ask_name;

impl ProjectView {
    /// Lists the contacts with a role in the project `project` by name.
    pub(super) fn show_people(&self, vault: &Vault, project: &ProjectSlug) {
        let list = &self.imp().people_list;
        list.remove_all();
        for contact in vault.contacts() {
            if let Some(role) = contact.role(project) {
                list.append(&person_row(vault, contact, role));
            }
        }
    }

    /// Applies `change` to a copy of the vault, then shows that on all
    /// pages, or tells what went wrong.
    fn change_contacts(&self, change: impl FnOnce(&mut Vault) -> Result<(), SaveError>) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        match change(&mut vault) {
            Ok(()) => {
                self.imp().vault.replace(Some(Rc::new(vault)));
                self.emit_by_name::<()>("vault-changed", &[]);
            }
            Err(err) => show_error(self, &gettext("Cannot Save Contact"), &err.to_string()),
        }
    }

    /// Sets the role of the contact `contact` in the project shown, or with
    /// `None` removes them from it.
    fn set_role(&self, contact: &ContactSlug, role: Option<&str>) {
        let Some(project) = self.slug() else {
            return;
        };
        self.change_contacts(|vault| {
            vault.update_contact(contact, |contact| {
                contact.set_role(&project, role);
                Ok(())
            })?;
            Ok(())
        });
    }

    /// Asks for a contact who does not work on the project yet and their
    /// role, and adds them.
    pub(super) async fn add_person(&self) {
        let Some(project) = self.slug() else {
            return;
        };
        let vault = self.vault();
        let contacts: Vec<&Contact> = vault
            .contacts()
            .iter()
            .filter(|contact| !contact.archived && contact.role(&project).is_none())
            .collect();
        if contacts.is_empty() {
            show_error(
                self,
                &gettext("No Contacts to Add"),
                &gettext("All contacts work on this project already. Add a new one instead."),
            );
            return;
        }
        let names: Vec<&str> = contacts
            .iter()
            .map(|contact| contact.name.as_str())
            .collect();
        let dropdown = gtk::DropDown::from_strings(&names);
        dropdown.set_enable_search(true);
        dropdown.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        dropdown.set_expression(Some(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<&gtk::Expression>,
            "string",
        )));
        let role = gtk::Entry::builder()
            .placeholder_text(gettext("Role"))
            .activates_default(true)
            .build();
        let fields = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .build();
        fields.append(&dropdown);
        fields.append(&role);
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Add Contact"))
            .extra_child(&fields)
            .close_response("cancel")
            .default_response("add")
            .focus_widget(&role)
            .build();
        dialog.add_responses(&[("cancel", &gettext("_Cancel")), ("add", &gettext("_Add"))]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        if dialog.choose_future(Some(self)).await != "add" {
            return;
        }
        let Some(contact) = contacts.get(dropdown.selected() as usize) else {
            return;
        };
        let role = role.text().trim().to_owned();
        self.set_role(&contact.slug, Some(&role));
    }

    /// Asks for the details of a new contact who works on the project.
    pub(super) fn new_person(&self) {
        let vault = self.vault();
        let dialog = ContactDialog::new(&vault, None, self.slug().as_ref());
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |dialog| {
                let mut vault = Vault::clone(&view.vault());
                match dialog.save_to(&mut vault) {
                    Ok(_) => {
                        dialog.close();
                        view.imp().vault.replace(Some(Rc::new(vault)));
                        view.emit_by_name::<()>("vault-changed", &[]);
                    }
                    Err(err) => {
                        show_error(dialog, &gettext("Cannot Save Contact"), &err.to_string());
                    }
                }
            }
        ));
        dialog.present(Some(self));
    }

    pub(super) async fn change_role(&self, contact: ContactSlug) {
        let Some(project) = self.slug() else {
            return;
        };
        let vault = self.vault();
        let Some(role) = vault.contact(&contact).and_then(|c| c.role(&project)) else {
            return;
        };
        let role = ask_name(
            self,
            &gettext("Change Role"),
            &gettext("_Change"),
            role,
            |_| true,
        )
        .await;
        if let Some(role) = role {
            self.set_role(&contact, Some(&role));
        }
    }

    pub(super) fn remove_person(&self, contact: &ContactSlug) {
        self.set_role(contact, None);
    }
}

/// A contact with `role` in the project, which opens the contact.
fn person_row(vault: &Vault, contact: &Contact, role: &str) -> adw::ActionRow {
    let target = contact.slug.to_string().to_variant();
    let row = adw::ActionRow::builder()
        .title(&contact.name)
        .subtitle(role)
        .use_markup(false)
        .activatable(true)
        .action_name("win.show-contact")
        .action_target(&target)
        .build();
    row.add_prefix(&avatar(vault.root(), contact, 32));
    let menu = gio::Menu::new();
    for (label, action) in [
        (gettext("_Change Role…"), "people.change-role"),
        (gettext("_Remove From Project"), "people.remove"),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target));
        menu.append_item(&item);
    }
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(gettext("More"))
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&button);
    row
}
