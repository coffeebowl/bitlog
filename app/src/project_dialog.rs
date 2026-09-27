use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gdk, gio, glib};
use knotbook_core::{EditError, Project, ProjectSlug, check_repo_path};

use crate::alert::show_error;
use crate::format::{PROJECT_STATUSES, status_name};
use crate::preferences_dialog::changed;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/project_dialog.ui")]
    pub struct ProjectDialog {
        /// The ID last made from the name, replaced along with the name
        /// until the user types another one.
        pub derived_slug: RefCell<String>,
        /// The project as the dialog showed it at first, to save only what
        /// the user changed.
        pub shown: RefCell<Option<Project>>,
        /// The repository folder as chosen, shown in `repo_row`.
        pub repo: RefCell<Option<PathBuf>>,
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
        #[template_child]
        pub repo_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub repo_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub clear_repo_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub choose_repo_button: TemplateChild<gtk::Button>,
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
            self.choose_repo_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    glib::spawn_future_local(async move { dialog.choose_repo().await });
                }
            ));
            self.clear_repo_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.set_repo(None)
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
        imp.shown.replace(Some(shown.clone()));
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

    /// Sets what the user changed of everything but the ID of `project`,
    /// so that changes made elsewhere meanwhile are kept.
    pub fn apply(&self, project: &mut Project) -> Result<(), EditError> {
        let imp = self.imp();
        let shown = imp.shown.borrow();
        let shown = shown.as_ref().expect("the dialog shows a project");
        changed(&mut project.name, &shown.name, self.name());
        let rgba = imp.color_button.rgba();
        let channel = |value: f32| (value * 255.0).round() as u8;
        let color = format!(
            "#{:02x}{:02x}{:02x}",
            channel(rgba.red()),
            channel(rgba.green()),
            channel(rgba.blue())
        );
        // Files may write the digits in upper case.
        if !color.eq_ignore_ascii_case(&shown.color) {
            project.set_color(&color)?;
        }
        let category = imp.category_row.text().trim().to_owned();
        changed(&mut project.category, &shown.category, category);
        let status = PROJECT_STATUSES[imp.status_row.selected() as usize];
        changed(&mut project.status, &shown.status, status);
        changed(
            &mut project.pinned,
            &shown.pinned,
            imp.pinned_row.is_active(),
        );
        Ok(())
    }

    /// Shows the repository `repo` of the project on this device, or why
    /// it cannot be read.
    pub fn show_repo(&self, repo: Result<Option<PathBuf>, String>) {
        let imp = self.imp();
        imp.repo_group.set_visible(true);
        match repo {
            Ok(repo) => self.set_repo(repo),
            Err(err) => {
                imp.repo_row.set_subtitle(&err);
                imp.repo_row.set_sensitive(false);
            }
        }
    }

    /// The repository folder as chosen.
    pub fn repo(&self) -> Option<PathBuf> {
        self.imp().repo.borrow().clone()
    }

    fn set_repo(&self, repo: Option<PathBuf>) {
        let imp = self.imp();
        let shown = repo
            .as_deref()
            .map_or_else(|| gettext("None"), |repo| repo.display().to_string());
        imp.repo_row.set_subtitle(&shown);
        imp.clear_repo_button.set_visible(repo.is_some());
        imp.repo.replace(repo);
    }

    async fn choose_repo(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose Repository"))
            .modal(true)
            .build();
        if let Some(repo) = self.repo() {
            dialog.set_initial_folder(Some(&gio::File::for_path(repo)));
        }
        let root = self.root().and_downcast::<gtk::Window>();
        // Dismissing the dialog is reported as an error, too.
        let Ok(folder) = dialog.select_folder_future(root.as_ref()).await else {
            return;
        };
        let checked = folder
            .path()
            .ok_or_else(|| gettext("Choose a folder on a local file system."))
            .and_then(|path| {
                check_repo_path(&path)
                    .map(|()| path)
                    .map_err(|err| err.to_string())
            });
        match checked {
            Ok(path) => self.set_repo(Some(path)),
            Err(message) => show_error(self, &gettext("Cannot Use Folder"), &message),
        }
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
