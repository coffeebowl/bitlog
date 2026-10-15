use std::cell::{Cell, RefCell};
use std::fs;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, ContactSlug, ProjectSlug, SaveError, Vault};
use chrono::{Datelike, Local, NaiveDate};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::glib;

use crate::format::{format_full_date, format_month};
use crate::widgets::{Choices, changed, param, set_class};

mod labeled;

use labeled::{Kind, Labeled, remove_button};

/// The zones of the system's time zone table, which GLib knows.
const ZONE_TABLE: &str = "/usr/share/zoneinfo/zone.tab";

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/contact_dialog.ui")]
    pub struct ContactDialog {
        /// The contact as the dialog showed it at first, to save only what
        /// the user changed. `None` for a new contact.
        pub shown: RefCell<Option<Contact>>,
        /// The ID last made from the name, replaced along with the name
        /// until the user types another one.
        pub derived_slug: RefCell<String>,
        /// The IDs and names of the contacts there are, which a new
        /// contact cannot take.
        pub taken: RefCell<Vec<(ContactSlug, String)>>,
        /// The slugs and names of the projects to choose from.
        pub projects: RefCell<Vec<(ProjectSlug, String)>>,
        /// The rows of phone numbers, e-mail addresses, other fields and
        /// roles, in their order.
        pub labeled: RefCell<Vec<Labeled>>,
        pub timezones: Choices<String>,
        /// Whether the system has a time zone table to choose from.
        pub has_timezones: Cell<bool>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub name_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub nickname_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub slug_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub slug_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub slug_hint: TemplateChild<gtk::Label>,
        #[template_child]
        pub phone_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub phone_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub email_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub email_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub details_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub company_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub hours_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub timezone_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub birthday_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub birthday_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub day_button: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub month_dropdown: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub year_button: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub note_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub note_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub projects_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub projects_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub add_field_button: TemplateChild<gtk::MenuButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactDialog {
        const NAME: &'static str = "BitLogContactDialog";
        type Type = super::ContactDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            for (action, kind) in [
                ("contact.add-phone", Kind::Phone),
                ("contact.add-email", Kind::Email),
                ("contact.add-field", Kind::Field),
                ("contact.add-role", Kind::Role),
            ] {
                klass.install_action_async(
                    action,
                    Some(glib::VariantTy::STRING),
                    move |dialog, _, label| async move {
                        let label: String = param(label.as_ref(), "labels");
                        dialog.add_labeled(kind, label).await;
                    },
                );
            }
            klass.install_action(
                "contact.show-field",
                Some(glib::VariantTy::STRING),
                |dialog, _, field| dialog.show_field(&param::<String>(field, "fields")),
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ContactDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted when the user wants to save; the dialog stays open
            // until it is closed.
            SIGNALS.get_or_init(|| vec![Signal::builder("save").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.cancel_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    dialog.close();
                }
            ));
            self.save_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.emit_by_name::<()>("save", &[])
            ));
            self.name_row.connect_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.name_changed()
            ));
            self.slug_row.connect_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.update_save_button()
            ));
            let months: Vec<String> = (1..=12)
                .map(|month| format_month(NaiveDate::from_ymd_opt(2000, month, 1).expect("valid")))
                .collect();
            let months: Vec<&str> = months.iter().map(String::as_str).collect();
            self.month_dropdown
                .set_model(Some(&gtk::StringList::new(&months)));
            self.day_button.connect_value_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.show_birthday()
            ));
            self.year_button.connect_value_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.show_birthday()
            ));
            self.month_dropdown.connect_selected_notify(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.show_birthday()
            ));
            // Search finds zones by any part of their name.
            self.timezone_row
                .set_expression(Some(gtk::PropertyExpression::new(
                    gtk::StringObject::static_type(),
                    None::<&gtk::Expression>,
                    "string",
                )));
            self.timezone_row
                .set_search_match_mode(gtk::StringFilterMatchMode::Substring);
            for row in [
                self.company_row.upcast_ref::<adw::PreferencesRow>(),
                self.hours_row.upcast_ref(),
                self.timezone_row.upcast_ref(),
                self.birthday_row.upcast_ref(),
            ] {
                let button = remove_button(glib::clone!(
                    #[weak]
                    dialog,
                    #[weak]
                    row,
                    move || {
                        row.set_visible(false);
                        dialog.update_menus();
                    }
                ));
                match row.downcast_ref::<adw::EntryRow>() {
                    Some(entry) => entry.add_suffix(&button),
                    None => row
                        .downcast_ref::<adw::ActionRow>()
                        .expect("the other fields are action rows")
                        .add_suffix(&button),
                }
            }
            self.note_group
                .set_header_suffix(Some(&remove_button(glib::clone!(
                    #[weak]
                    dialog,
                    move || {
                        dialog.imp().note_group.set_visible(false);
                        dialog.update_menus();
                    }
                ))));
        }
    }

    impl WidgetImpl for ContactDialog {}
    impl AdwDialogImpl for ContactDialog {}
}

glib::wrapper! {
    /// Asks for the details of a new contact or changes those of one.
    pub struct ContactDialog(ObjectSubclass<imp::ContactDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ContactDialog {
    /// A dialog to change `contact` of `vault`, or to add a new contact if
    /// it is `None`, who works on `project` if given.
    pub fn new(vault: &Vault, contact: Option<&Contact>, project: Option<&ProjectSlug>) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.projects.replace(
            vault
                .projects()
                .iter()
                .map(|project| (project.slug.clone(), project.name.clone()))
                .collect(),
        );
        imp.taken.replace(
            vault
                .contacts()
                .iter()
                .map(|contact| (contact.slug.clone(), contact.name.clone()))
                .collect(),
        );
        let new = Contact::new("new".parse().expect("valid slug"), "");
        let shown = contact.unwrap_or(&new);
        imp.name_row.set_text(&shown.name);
        if contact.is_some() {
            dialog.set_title(&gettext("Edit Contact"));
            imp.save_button.set_label(&gettext("_Save"));
            imp.slug_group.set_visible(false);
        } else {
            dialog.set_title(&gettext("New Contact"));
            imp.save_button.set_label(&gettext("_Add"));
        }
        for (kind, texts) in [
            (Kind::Phone, &shown.phones),
            (Kind::Email, &shown.emails),
            (Kind::Field, &shown.fields),
        ] {
            for (label, text) in texts {
                dialog.add_row(kind, label, text);
            }
        }
        // There is always room for a number and an address.
        if shown.phones.is_empty() {
            dialog.add_row(Kind::Phone, "work", "");
        }
        if shown.emails.is_empty() {
            dialog.add_row(Kind::Email, "work", "");
        }
        for (slug, role) in &shown.roles {
            dialog.add_row(Kind::Role, slug.as_str(), role);
        }
        if let Some(project) = project {
            dialog.add_row(Kind::Role, project.as_str(), "");
        }
        imp.nickname_row.set_text(&shown.nickname);
        imp.company_row.set_text(&shown.company);
        imp.company_row.set_visible(!shown.company.is_empty());
        imp.hours_row.set_text(&shown.hours);
        imp.hours_row.set_visible(!shown.hours.is_empty());
        dialog.offer_timezones(&shown.timezone);
        imp.timezone_row.set_visible(!shown.timezone.is_empty());
        dialog.set_birthday(shown.birthday.unwrap_or_else(|| Local::now().date_naive()));
        imp.birthday_row.set_visible(shown.birthday.is_some());
        imp.note_view.buffer().set_text(&shown.note);
        imp.note_group.set_visible(!shown.note.is_empty());
        imp.shown.replace(contact.cloned());
        dialog.update_menus();
        dialog.update_save_button();
        dialog
    }

    pub fn connect_save(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "save",
            false,
            glib::closure_local!(move |dialog: &Self| callback(dialog)),
        );
    }

    fn name(&self) -> String {
        self.imp().name_row.text().trim().to_owned()
    }

    /// The ID of a new contact, `None` if the one entered is invalid.
    fn slug(&self) -> Option<ContactSlug> {
        self.imp().slug_row.text().trim().parse().ok()
    }

    /// Saves what the dialog holds to the contact it shows in `vault`, or
    /// as a new contact. Returns the slug of the contact saved.
    pub fn save_to(&self, vault: &mut Vault) -> Result<ContactSlug, SaveError> {
        let shown = self.imp().shown.borrow().clone();
        let saved = match shown {
            Some(shown) => vault.update_contact(&shown.slug, |contact| {
                self.apply(contact, &shown);
                Ok(())
            })?,
            None => {
                let slug = self.slug().expect("new contacts are saved with a valid ID");
                let new = Contact::new(slug.clone(), "");
                let mut contact = new.clone();
                self.apply(&mut contact, &new);
                vault.add_contact(contact)?
            }
        };
        Ok(saved.slug.clone())
    }

    /// Sets what the user changed of everything but the ID of `contact`,
    /// compared with `shown`, so that changes made elsewhere meanwhile are
    /// kept.
    fn apply(&self, contact: &mut Contact, shown: &Contact) {
        let imp = self.imp();
        changed(&mut contact.name, &shown.name, self.name());
        let text = |row: &adw::EntryRow| {
            let text = row.text().trim().to_owned();
            if row.is_visible() {
                text
            } else {
                String::new()
            }
        };
        let nickname = imp.nickname_row.text().trim().to_owned();
        changed(&mut contact.nickname, &shown.nickname, nickname);
        changed(&mut contact.company, &shown.company, text(&imp.company_row));
        changed(&mut contact.hours, &shown.hours, text(&imp.hours_row));
        let timezone = imp
            .timezones
            .chosen(&*imp.timezone_row)
            .filter(|_| imp.timezone_row.is_visible())
            .unwrap_or_default();
        changed(&mut contact.timezone, &shown.timezone, timezone);
        let birthday = imp.birthday_row.is_visible().then(|| self.birthday());
        changed(&mut contact.birthday, &shown.birthday, birthday);
        let buffer = imp.note_view.buffer();
        let (start, end) = buffer.bounds();
        let note = buffer.text(&start, &end, false).trim().to_owned();
        let note = if imp.note_group.is_visible() {
            note
        } else {
            String::new()
        };
        changed(&mut contact.note, &shown.note, note);
        changed(&mut contact.phones, &shown.phones, self.texts(Kind::Phone));
        changed(&mut contact.emails, &shown.emails, self.texts(Kind::Email));
        changed(&mut contact.fields, &shown.fields, self.texts(Kind::Field));
        let roles = self
            .texts(Kind::Role)
            .into_iter()
            .map(|(slug, role)| (slug.parse().expect("roles are labeled by slugs"), role))
            .collect();
        changed(&mut contact.roles, &shown.roles, roles);
    }

    /// Shows the field `field` that was hidden, for the user to fill in.
    fn show_field(&self, field: &str) {
        let imp = self.imp();
        // A hidden group hides the rows shown in it right away again.
        if field != "note" {
            imp.details_group.set_visible(true);
        }
        match field {
            "company" => imp.company_row.set_visible(true),
            "hours" => imp.hours_row.set_visible(true),
            "timezone" => imp.timezone_row.set_visible(true),
            "birthday" => imp.birthday_row.set_visible(true),
            _ => imp.note_group.set_visible(true),
        }
        self.update_menus();
        match field {
            "company" => imp.company_row.grab_focus(),
            "hours" => imp.hours_row.grab_focus(),
            "note" => imp.note_view.grab_focus(),
            "timezone" => imp.timezone_row.grab_focus(),
            _ => {
                imp.birthday_button.popup();
                true
            }
        };
    }

    /// Offers the time zones of the system, with `current` chosen, or the
    /// zone of this device if there is none.
    fn offer_timezones(&self, current: &str) {
        let imp = self.imp();
        let mut zones: Vec<String> = fs::read_to_string(ZONE_TABLE)
            .map(|table| {
                table
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .filter_map(|line| line.split('\t').nth(2))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        zones.sort();
        imp.has_timezones.set(!zones.is_empty());
        let selected = if current.is_empty() {
            glib::TimeZone::local().identifier().to_string()
        } else {
            current.to_owned()
        };
        // A zone written by hand that the table lacks stays as it is.
        if !current.is_empty() && !zones.contains(&selected) {
            zones.insert(0, selected.clone());
        }
        imp.timezones
            .fill(&*imp.timezone_row, zones, &selected, Clone::clone);
    }

    fn birthday(&self) -> NaiveDate {
        let imp = self.imp();
        let year = imp.year_button.value_as_int();
        // While the date is set, the fields may hold no valid value yet.
        let month = imp.month_dropdown.selected().min(11) + 1;
        // February 30 becomes February 28 or 29.
        let day = u32::try_from(imp.day_button.value_as_int()).map_or(1, |day| day.max(1));
        (1..=day)
            .rev()
            .find_map(|day| NaiveDate::from_ymd_opt(year, month, day))
            .expect("every month has a first day")
    }

    fn set_birthday(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.year_button.set_value(f64::from(date.year()));
        imp.month_dropdown.set_selected(date.month0());
        imp.day_button.set_value(f64::from(date.day()));
        self.show_birthday();
    }

    fn show_birthday(&self) {
        let imp = self.imp();
        imp.birthday_button
            .set_label(&format_full_date(self.birthday()));
    }
    fn name_changed(&self) {
        let imp = self.imp();
        if imp.slug_group.is_visible() && imp.slug_row.text() == *imp.derived_slug.borrow() {
            // Short, as mentions use it: the first name.
            let first = self.name();
            let first = first.split_whitespace().next().unwrap_or_default();
            let slug = ContactSlug::from_name(first)
                .map(|slug| slug.to_string())
                .unwrap_or_default();
            imp.derived_slug.replace(slug.clone());
            imp.slug_row.set_text(&slug);
        }
        self.update_save_button();
    }

    /// Allows saving once there is a name and, for a new contact, a valid
    /// ID no other contact has.
    fn update_save_button(&self) {
        let imp = self.imp();
        let slug = self.slug();
        let taken = imp.taken.borrow();
        let owner = slug
            .as_ref()
            .and_then(|slug| taken.iter().find(|(other, _)| other == slug))
            .map(|(_, name)| name);
        let slug_valid = !imp.slug_group.is_visible() || (slug.is_some() && owner.is_none());
        let slug_error = !slug_valid && !imp.slug_row.text().is_empty();
        set_class(&*imp.slug_row, "error", slug_error);
        let hint = match owner {
            Some(name) => {
                gettext("The contact “{name}” already has this ID.").replace("{name}", name)
            }
            None => gettext("The ID can only be changed later on the command line."),
        };
        imp.slug_hint.set_label(&hint);
        set_class(&*imp.slug_hint, "error", owner.is_some());
        set_class(&*imp.slug_hint, "warning", owner.is_none());
        imp.save_button
            .set_sensitive(!self.name().is_empty() && slug_valid);
    }
}
