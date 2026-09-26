use std::cell::RefCell;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gdk, glib};
use knotbook_core::{EditError, Project, ProjectSlug};

use crate::format::{PROJECT_STATUSES, status_name};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/project_dialog.ui")]
    pub struct ProjectDialog {
        /// The ID last made from the name, replaced along with the name
        /// until the user types another one.
        pub derived_slug: RefCell<String>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub name_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub slug_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub slug_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub color_button: TemplateChild<gtk::ColorDialogButton>,
        #[template_child]
        pub category_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub status_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub pinned_row: TemplateChild<adw::SwitchRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectDialog {
        const NAME: &'static str = "KnotbookProjectDialog";
        type Type = super::ProjectDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted when the user wants to save; the dialog stays open
            // until it is closed.
            SIGNALS.get_or_init(|| vec![Signal::builder("save").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            let names: Vec<String> = PROJECT_STATUSES.into_iter().map(status_name).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            self.status_row
                .set_model(Some(&gtk::StringList::new(&names)));
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
        }
    }

    impl WidgetImpl for ProjectDialog {}
    impl AdwDialogImpl for ProjectDialog {}
}

glib::wrapper! {
    /// Asks for the details of a new project or changes those of one.
    pub struct ProjectDialog(ObjectSubclass<imp::ProjectDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ProjectDialog {
    /// A dialog to change `project`, or to add a new project if it is `None`.
    pub fn new(project: Option<&Project>) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        // A new project starts with the values of the core.
        let today = chrono::Local::now().date_naive();
        let new = Project::new("new".parse().expect("valid slug"), "", today);
        let shown = project.unwrap_or(&new);
        imp.name_row.set_text(&shown.name);
        imp.color_button
            .set_rgba(&gdk::RGBA::parse(shown.color.as_str()).expect("project colors are valid"));
        imp.category_row.set_text(&shown.category);
        let status = PROJECT_STATUSES
            .iter()
            .position(|status| *status == shown.status)
            .expect("every status is listed");
        imp.status_row
            .set_selected(u32::try_from(status).expect("few statuses"));
        imp.pinned_row.set_active(shown.pinned);
        if project.is_some() {
            dialog.set_title(&gettext("Edit Project"));
            imp.save_button.set_label(&gettext("_Save"));
            imp.slug_group.set_visible(false);
        } else {
            dialog.set_title(&gettext("New Project"));
            imp.save_button.set_label(&gettext("_Add"));
        }
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

    pub fn name(&self) -> String {
        self.imp().name_row.text().trim().to_owned()
    }

    /// The ID of a new project, `None` if the one entered is invalid.
    pub fn slug(&self) -> Option<ProjectSlug> {
        self.imp().slug_row.text().trim().parse().ok()
    }

    /// Sets everything but the ID of `project` as entered.
    pub fn apply(&self, project: &mut Project) -> Result<(), EditError> {
        let imp = self.imp();
        project.name = self.name();
        let rgba = imp.color_button.rgba();
        let channel = |value: f32| (value * 255.0).round() as u8;
        project.set_color(&format!(
            "#{:02x}{:02x}{:02x}",
            channel(rgba.red()),
            channel(rgba.green()),
            channel(rgba.blue())
        ))?;
        project.category = imp.category_row.text().trim().to_owned();
        project.status = PROJECT_STATUSES[imp.status_row.selected() as usize];
        project.pinned = imp.pinned_row.is_active();
        Ok(())
    }

    fn name_changed(&self) {
        let imp = self.imp();
        if imp.slug_group.is_visible() && imp.slug_row.text() == *imp.derived_slug.borrow() {
            let slug = ProjectSlug::from_name(&self.name())
                .map(|slug| slug.to_string())
                .unwrap_or_default();
            imp.derived_slug.replace(slug.clone());
            imp.slug_row.set_text(&slug);
        }
        self.update_save_button();
    }

    /// Allows saving once there is a name and, for a new project, a valid ID.
    fn update_save_button(&self) {
        let imp = self.imp();
        let slug_valid = !imp.slug_group.is_visible() || self.slug().is_some();
        let slug_error = !slug_valid && !imp.slug_row.text().is_empty();
        if slug_error {
            imp.slug_row.add_css_class("error");
        } else {
            imp.slug_row.remove_css_class("error");
        }
        imp.save_button
            .set_sensitive(!self.name().is_empty() && slug_valid);
    }
}
