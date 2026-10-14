use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, ContactSlug, Vault};
use chrono::NaiveTime;
use gettextrs::gettext;
use gtk::glib;

use crate::alert::toast_overlay;
use crate::colors::color_dot;
use crate::contact_image::picture;
use crate::format::{contact_label, format_full_date, format_time};
use crate::launch;
use crate::note_view::backlink_row;
use crate::search_index::SearchIndex;
use crate::window::show_action;

/// How many places that mention the contact are listed at once.
const MENTIONS_AT_ONCE: u32 = 10;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/contact_view.ui")]
    pub struct ContactView {
        /// The contact shown.
        pub slug: RefCell<Option<ContactSlug>>,
        /// The rows added to the groups, removed before showing a contact.
        pub rows: RefCell<Vec<(adw::PreferencesGroup, adw::ActionRow)>>,
        #[template_child]
        pub scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub picture_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
        #[template_child]
        pub name_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub company_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub phone_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub email_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub details_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub projects_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub mentions_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub mentions_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub more_button: TemplateChild<gtk::Button>,
        pub index: RefCell<SearchIndex>,
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// Counts the lookups of mentions, so that one finishing after a
        /// newer one started is dropped.
        pub lookups: Cell<u32>,
        pub mentions_shown: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactView {
        const NAME: &'static str = "BitLogContactView";
        type Type = super::ContactView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ContactView {
        fn constructed(&self) {
            self.parent_constructed();
            self.more_button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.show_mentions(false)
            ));
        }
    }
    impl WidgetImpl for ContactView {}
    impl NavigationPageImpl for ContactView {}
}

glib::wrapper! {
    /// One contact with the fields that have a value, the projects they
    /// work on and the places that mention them.
    pub struct ContactView(ObjectSubclass<imp::ContactView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ContactView {
    /// The contact shown, if any.
    pub fn slug(&self) -> Option<ContactSlug> {
        self.imp().slug.borrow().clone()
    }

    /// The button with the picture, which takes dropped pictures.
    pub fn picture_button(&self) -> gtk::MenuButton {
        self.imp().picture_button.clone()
    }

    pub fn set_index(&self, index: SearchIndex) {
        self.imp().index.replace(index);
    }

    /// Shows `contact` of `vault`.
    pub fn show(&self, vault: &Rc<Vault>, contact: &Contact) {
        let imp = self.imp();
        imp.vault.replace(Some(vault.clone()));
        if imp.slug.borrow().as_ref() != Some(&contact.slug) {
            imp.scrolled.vadjustment().set_value(0.0);
        }
        imp.slug.replace(Some(contact.slug.clone()));
        self.set_title(&contact.name);
        imp.avatar.set_text(Some(&contact.name));
        let picture = picture(vault.root(), &contact.slug);
        imp.avatar.set_custom_image(picture.as_ref());
        imp.name_label.set_label(&contact.name);
        imp.company_label.set_label(&contact.company);
        imp.company_label.set_visible(!contact.company.is_empty());

        for (group, row) in imp.rows.take() {
            group.remove(&row);
        }
        for (label, number) in &contact.phones {
            let uri = phone_uri(number);
            self.add_row(&imp.phone_group, &contact_label(label), number, Some(uri));
        }
        for (label, address) in &contact.emails {
            let uri = mail_uri(address);
            self.add_row(&imp.email_group, &contact_label(label), address, Some(uri));
        }
        let details = &imp.details_group;
        self.add_text(details, &gettext("Working Hours"), &contact.hours);
        if !contact.timezone.is_empty() {
            let zone = match local_time(&contact.timezone) {
                // Translators: A time zone and the time there, as in
                // "Europe/Berlin, now 14:30".
                Some(time) => gettext("{zone}, now {time}")
                    .replace("{zone}", &contact.timezone)
                    .replace("{time}", &time),
                None => contact.timezone.clone(),
            };
            self.add_text(details, &gettext("Time Zone"), &zone);
        }
        if let Some(birthday) = contact.birthday {
            self.add_text(details, &gettext("Birthday"), &format_full_date(birthday));
        }
        for (label, text) in &contact.fields {
            let web = text.starts_with("https://") || text.starts_with("http://");
            self.add_row(details, label, text, web.then(|| text.clone()));
        }
        self.add_text(details, &gettext("Note"), &contact.note);
        for (slug, role) in &contact.roles {
            let row = adw::ActionRow::builder()
                .title(vault.project_name(slug))
                .subtitle(role.as_str())
                .use_markup(false)
                .build();
            // A project the vault lacks has no page to go to.
            if let Some(project) = vault.project(slug) {
                let dot = gtk::Label::builder()
                    .label(color_dot(&project.color))
                    .use_markup(true)
                    .build();
                row.add_prefix(&dot);
                row.set_action_target_value(Some(&slug.to_string().to_variant()));
                row.set_action_name(Some("win.show-project"));
                row.set_activatable(true);
                row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            }
            self.append(&imp.projects_group, row);
        }
        for group in [
            &*imp.phone_group,
            &imp.email_group,
            &imp.details_group,
            &imp.projects_group,
        ] {
            let has_rows = imp.rows.borrow().iter().any(|(other, _)| other == group);
            group.set_visible(has_rows);
        }
        self.show_mentions(true);
    }

    /// Looks up the places that mention the contact shown in the background
    /// and lists them, all over again if `first`, else the next batch.
    fn show_mentions(&self, first: bool) {
        let imp = self.imp();
        let (Some(slug), Some(vault)) = (self.slug(), imp.vault.borrow().clone()) else {
            return;
        };
        let lookup = imp.lookups.get() + u32::from(first);
        imp.lookups.set(lookup);
        let skip = if first { 0 } else { imp.mentions_shown.get() };
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let mentions = index.mentions(&vault, slug, skip, MENTIONS_AT_ONCE).await;
                let imp = view.imp();
                if imp.lookups.get() != lookup {
                    return;
                }
                let mentions = mentions.unwrap_or_else(|err| {
                    glib::g_warning!("bitlog", "{err}");
                    Vec::new()
                });
                if first {
                    imp.mentions_list.remove_all();
                }
                for mention in &mentions {
                    let row = backlink_row(&vault, mention);
                    let (action, target) = show_action(&mention.found);
                    row.set_action_target_value(Some(&target));
                    row.set_action_name(Some(action));
                    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                    imp.mentions_list.append(&row);
                }
                let added = u32::try_from(mentions.len()).expect("at most a batch comes");
                imp.mentions_shown.set(skip + added);
                imp.mentions_group.set_visible(skip + added > 0);
                // A full batch may have more behind it.
                imp.more_button.set_visible(added == MENTIONS_AT_ONCE);
            }
        ));
    }

    fn append(&self, group: &adw::PreferencesGroup, row: adw::ActionRow) {
        group.add(&row);
        self.imp().rows.borrow_mut().push((group.clone(), row));
    }

    /// Adds a row with `text` below `title` to `group`, unless `text` is
    /// empty. It can be selected, but not copied with a button.
    fn add_text(&self, group: &adw::PreferencesGroup, title: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let row = property_row(title, text);
        row.set_subtitle_selectable(true);
        self.append(group, row);
    }

    /// Adds a row with `text` below `title` to `group`, with a button to
    /// copy it. With `uri`, activating the row opens it.
    fn add_row(&self, group: &adw::PreferencesGroup, title: &str, text: &str, uri: Option<String>) {
        let row = property_row(title, text);
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(gettext("Copy"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let text = text.to_owned();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&text);
            toast_overlay(button).add_toast(adw::Toast::new(&gettext("Copied to clipboard")));
        });
        match uri {
            Some(uri) => {
                row.set_activatable(true);
                row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
                row.connect_activated(move |row| {
                    let (row, uri) = (row.clone(), uri.clone());
                    glib::spawn_future_local(async move { launch::open_uri(&row, &uri).await });
                });
            }
            None => row.set_subtitle_selectable(true),
        }
        // Last, so that the buttons of all rows line up.
        row.add_suffix(&copy);
        self.append(group, row);
    }
}

/// The address that calls the phone number `number`.
pub fn phone_uri(number: &str) -> String {
    format!("tel:{}", number.replace(' ', ""))
}

/// The address that writes to the e-mail address `address`.
pub fn mail_uri(address: &str) -> String {
    format!("mailto:{address}")
}

fn property_row(title: &str, text: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(text)
        .use_markup(false)
        .css_classes(["property"])
        .build()
}

/// The time now in the time zone `timezone`, an IANA name, if GLib knows
/// it.
fn local_time(timezone: &str) -> Option<String> {
    let zone = glib::TimeZone::from_identifier(Some(timezone))?;
    let now = glib::DateTime::now(&zone).ok()?;
    let time = NaiveTime::from_hms_opt(
        now.hour().try_into().ok()?,
        now.minute().try_into().ok()?,
        0,
    )?;
    Some(format_time(time))
}
