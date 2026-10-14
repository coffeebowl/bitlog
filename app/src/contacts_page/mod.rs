use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, ContactSlug, SaveError, Vault};
use gettextrs::{gettext, ngettext};
use glib::subclass::Signal;
use gtk::{gdk, gio, glib};

use crate::alert::show_error;
use crate::contact_dialog::ContactDialog;
use crate::contact_image::{Source, avatar};
use crate::contact_view::ContactView;
use crate::format::plural;
use crate::projects_page::shows;
use crate::search_index::SearchIndex;
use crate::trash::trash;
use crate::widgets::param;

mod pictures;

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/contacts_page.ui")]
    pub struct ContactsPage {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The groups of the list, the archived contacts in the last.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        /// Pushed on top of the list, owned here because it is only in the
        /// navigation view while shown.
        pub contact_view: ContactView,
        #[template_child]
        pub nav: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub list: TemplateChild<adw::PreferencesPage>,
        #[template_child]
        pub overview: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
    }

    impl Default for ContactsPage {
        fn default() -> Self {
            Self {
                vault: RefCell::default(),
                groups: RefCell::default(),
                contact_view: glib::Object::new(),
                nav: TemplateChild::default(),
                stack: TemplateChild::default(),
                list: TemplateChild::default(),
                overview: TemplateChild::default(),
                search_entry: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactsPage {
        const NAME: &'static str = "BitLogContactsPage";
        type Type = super::ContactsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("contacts.add", None, |page, _, _| page.edit(None));
            klass.install_action("contacts.edit", None, |page, _, _| {
                page.edit(page.imp().contact_view.slug());
            });
            klass.install_action(
                "contacts.open",
                Some(glib::VariantTy::STRING),
                |page, _, slug| page.open_contact(&param(slug, "contacts")),
            );
            klass.install_action("contacts.archive", None, |page, _, _| {
                page.set_archived(true);
            });
            klass.install_action("contacts.unarchive", None, |page, _, _| {
                page.set_archived(false);
            });
            klass.install_action_async("contacts.trash", None, |page, _, _| async move {
                page.trash_contact().await;
            });
            klass.install_action_async("contacts.choose-picture", None, |page, _, _| async move {
                page.choose_picture().await
            });
            klass.install_action_async("contacts.paste-picture", None, |page, _, _| async move {
                page.paste_picture().await
            });
            klass.install_action_async("contacts.remove-picture", None, |page, _, _| async move {
                page.remove_picture().await
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ContactsPage {
        fn constructed(&self) {
            self.parent_constructed();
            let page = self.obj();
            let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
            drop.connect_drop(glib::clone!(
                #[weak]
                page,
                #[upgrade_or]
                false,
                move |_, value, _, _| {
                    let Some(path) = value
                        .get::<gdk::FileList>()
                        .ok()
                        .and_then(|files| files.files().first()?.path())
                    else {
                        return false;
                    };
                    glib::spawn_future_local(async move {
                        page.set_picture(Source::File(path)).await;
                    });
                    true
                }
            ));
            self.contact_view.picture_button().add_controller(drop);
            self.search_entry.connect_search_changed(glib::clone!(
                #[weak]
                page,
                move |_| page.show_list()
            ));
            // Typing on the list searches.
            self.search_entry
                .set_key_capture_widget(Some(&*self.overview));
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                // Emitted after a contact was changed, with the changed
                // vault available through `vault()`, which the window then
                // shows on all pages, this one too.
                vec![Signal::builder("vault-changed").build()]
            })
        }
    }

    impl WidgetImpl for ContactsPage {}
    impl NavigationPageImpl for ContactsPage {}
}

glib::wrapper! {
    /// All contacts of the vault, to add, edit and open one.
    pub struct ContactsPage(ObjectSubclass<imp::ContactsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ContactsPage {
    /// Shows the contacts of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    pub fn set_index(&self, index: SearchIndex) {
        self.imp().contact_view.set_index(index);
    }

    pub fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the page is only shown with a vault")
    }

    pub fn connect_vault_changed(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "vault-changed",
            false,
            glib::closure_local!(move |page: &Self| callback(page)),
        );
    }

    /// Shows the contacts of the vault and the contact open as they are
    /// now.
    pub fn reload(&self) {
        let imp = self.imp();
        self.show_list();
        if shows(&imp.nav, "contact") && !self.show_contact_again() {
            imp.nav.pop_to_tag("contacts");
        }
    }

    /// Goes back to the list of contacts.
    pub fn show_overview(&self) {
        self.imp().nav.pop_to_tag("contacts");
    }

    /// Shows the contact `slug`, from where going back leads to the list.
    pub fn open_contact(&self, slug: &ContactSlug) {
        let imp = self.imp();
        let vault = self.vault();
        let Some(contact) = vault.contact(slug) else {
            return;
        };
        self.show_contact(&vault, contact);
        if !shows(&imp.nav, "contact") {
            imp.nav.push(&imp.contact_view);
        }
    }

    fn show_contact(&self, vault: &Rc<Vault>, contact: &Contact) {
        self.imp().contact_view.show(vault, contact);
        self.action_set_enabled("contacts.archive", !contact.archived);
        self.action_set_enabled("contacts.unarchive", contact.archived);
        let picture = Contact::image_path(vault.root(), &contact.slug);
        self.action_set_enabled("contacts.remove-picture", picture.is_file());
    }

    /// Shows the contact open as it is now. Returns whether it is still
    /// there.
    fn show_contact_again(&self) -> bool {
        let vault = self.vault();
        let contact = self
            .imp()
            .contact_view
            .slug()
            .and_then(|slug| vault.contact(&slug).cloned());
        if let Some(contact) = &contact {
            self.show_contact(&vault, contact);
        }
        contact.is_some()
    }

    /// Lists the contacts that match the search by name, the archived ones
    /// folded away at the end, unless the search found some of them.
    fn show_list(&self) {
        let imp = self.imp();
        for group in imp.groups.take() {
            imp.list.remove(&group);
        }
        let vault = self.vault();
        let search = imp.search_entry.text().trim().to_lowercase();
        let (archived, active): (Vec<&Contact>, Vec<&Contact>) = vault
            .contacts()
            .iter()
            .filter(|contact| matches(&vault, contact, &search))
            .partition(|contact| contact.archived);
        let mut groups = Vec::new();
        if !active.is_empty() {
            let group = adw::PreferencesGroup::new();
            for contact in active {
                group.add(&contact_row(&vault, contact));
            }
            groups.push(group);
        }
        if !archived.is_empty() {
            let expander = adw::ExpanderRow::builder()
                .title(gettext("Archived"))
                .subtitle(
                    ngettext(
                        "{count} contact",
                        "{count} contacts",
                        plural(archived.len()),
                    )
                    .replace("{count}", &archived.len().to_string()),
                )
                .expanded(!search.is_empty())
                .build();
            for contact in archived {
                expander.add_row(&contact_row(&vault, contact));
            }
            let group = adw::PreferencesGroup::new();
            group.add(&expander);
            groups.push(group);
        }
        for group in &groups {
            imp.list.add(group);
        }
        let shown = match (groups.is_empty(), vault.contacts().is_empty()) {
            (false, _) => "list",
            (true, true) => "empty",
            (true, false) => "no-results",
        };
        imp.stack.set_visible_child_name(shown);
        imp.groups.replace(groups);
    }

    /// Applies `change` to a copy of the vault, then shows that on all
    /// pages, or tells what went wrong under `heading`.
    fn change(&self, heading: &str, change: impl FnOnce(&mut Vault) -> Result<(), SaveError>) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        match change(&mut vault) {
            Ok(()) => {
                self.set_vault(Rc::new(vault));
                self.emit_by_name::<()>("vault-changed", &[]);
            }
            Err(err) => show_error(self, heading, &err.to_string()),
        }
    }

    fn set_archived(&self, archived: bool) {
        let Some(slug) = self.imp().contact_view.slug() else {
            return;
        };
        self.change(&gettext("Cannot Save Contact"), |vault| {
            vault.update_contact(&slug, |contact| {
                contact.archived = archived;
                Ok(())
            })?;
            Ok(())
        });
    }

    /// Asks for the details of the contact `slug`, or of a new contact.
    fn edit(&self, slug: Option<ContactSlug>) {
        let vault = self.vault();
        let contact = slug.as_ref().and_then(|slug| vault.contact(slug));
        let dialog = ContactDialog::new(&vault, contact, None);
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |dialog| {
                let mut vault = Vault::clone(&page.vault());
                match dialog.save_to(&mut vault) {
                    Ok(slug) => {
                        dialog.close();
                        page.set_vault(Rc::new(vault));
                        page.emit_by_name::<()>("vault-changed", &[]);
                        page.open_contact(&slug);
                    }
                    Err(err) => {
                        show_error(dialog, &gettext("Cannot Save Contact"), &err.to_string());
                    }
                }
            }
        ));
        dialog.present(Some(self));
    }

    /// Moves the contact shown to the trash, with their picture.
    async fn trash_contact(&self) {
        let Some(slug) = self.imp().contact_view.slug() else {
            return;
        };
        let vault = self.vault();
        let path = Contact::path(vault.root(), &slug);
        vault.record_removal(&path);
        if !trash(self, &path, &gettext("Contact moved to trash")).await {
            return;
        }
        // Gone for good if there is no trash, as the user agreed to above.
        let picture = gio::File::for_path(Contact::image_path(vault.root(), &slug));
        if picture.query_exists(gio::Cancellable::NONE)
            && picture.trash_future(glib::Priority::DEFAULT).await.is_err()
        {
            let _ = picture.delete_future(glib::Priority::DEFAULT).await;
        }
        self.show_overview();
        self.change(&gettext("Cannot Read Contacts"), |vault| {
            vault.reload_contacts()?;
            Ok(())
        });
    }
}

/// Whether `contact` of `vault` has `search`, in lower case, in their name,
/// company or slug, or in a project they work on or their role there.
fn matches(vault: &Vault, contact: &Contact, search: &str) -> bool {
    let has = |text: &str| text.to_lowercase().contains(search);
    has(&contact.name)
        || has(&contact.company)
        || has(contact.slug.as_str())
        || contact
            .roles
            .iter()
            .any(|(project, role)| has(vault.project_name(project)) || has(role))
}

/// A contact in the list, with their picture and company.
fn contact_row(vault: &Vault, contact: &Contact) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&contact.name)
        .subtitle(&contact.company)
        .use_markup(false)
        .activatable(true)
        .action_name("contacts.open")
        .action_target(&contact.slug.to_string().to_variant())
        .build();
    row.add_prefix(&avatar(vault.root(), contact, 32));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn search_finds_names_companies_and_roles() {
        let vault =
            Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/sample-vault"))
                .unwrap();
        let found = |search: &str| -> Vec<&str> {
            vault
                .contacts()
                .iter()
                .filter(|contact| matches(&vault, contact, search))
                .map(|contact| contact.slug.as_str())
                .collect()
        };
        assert_eq!(found(""), ["alex", "kim", "sam"]);
        assert_eq!(found("meyer"), ["kim"]);
        assert_eq!(found("example gmbh"), ["kim"]);
        assert_eq!(found("product owner"), ["kim"]);
        assert_eq!(found("infrastructure"), ["alex"]);
        assert!(found("nobody").is_empty());
    }
}
