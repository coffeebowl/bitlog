use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{EditError, Project, ProjectCategory, ProjectSlug, check_repo_path};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::colors;
use crate::format::{PROJECT_STATUSES, project_category_name, project_status_name};
use crate::widgets::{changed, set_class};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/project_dialog.ui")]
    pub struct ProjectDialog {
        /// The ID last made from the name, replaced along with the name
        /// until the user types another one.
        pub derived_slug: RefCell<String>,
        /// The project as the dialog showed it at first, to save only what
        /// the user changed.
        pub shown: RefCell<Option<Project>>,
        /// The repository folder as chosen, shown in `repo_row`.
        pub repo: RefCell<Option<PathBuf>>,
        /// The IDs and names of the projects there are, which a new
        /// project cannot take.
        pub taken: RefCell<Vec<(ProjectSlug, String)>>,
        /// One for each category, in the order of `ProjectCategory::ALL`.
        pub category_buttons: RefCell<Vec<gtk::CheckButton>>,
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
        pub slug_hint: TemplateChild<gtk::Label>,
        #[template_child]
        pub color_button: TemplateChild<gtk::ColorDialogButton>,
        #[template_child]
        pub category_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub status_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub repo_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub clear_repo_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub choose_repo_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectDialog {
        const NAME: &'static str = "BitLogProjectDialog";
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
            let names: Vec<String> = PROJECT_STATUSES
                .into_iter()
                .map(project_status_name)
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            self.status_row
                .set_model(Some(&gtk::StringList::new(&names)));
            dialog.add_category_rows();
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
    /// A dialog to change `project`, or to add a new project if it is
    /// `None`, next to the projects `projects`.
    pub fn new(project: Option<&Project>, projects: &[Project]) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        // A new project starts with the values of the core.
        let today = chrono::Local::now().date_naive();
        let new = Project::new("new".parse().expect("valid slug"), "", today);
        let shown = project.unwrap_or(&new);
        imp.name_row.set_text(&shown.name);
        imp.color_button.set_rgba(&colors::parse(&shown.color));
        dialog.show_category(shown);
        let status = PROJECT_STATUSES
            .iter()
            .position(|status| *status == shown.status)
            .expect("every status is listed");
        imp.status_row
            .set_selected(u32::try_from(status).expect("few statuses"));
        if project.is_some() {
            dialog.set_title(&gettext("Edit Project"));
            imp.save_button.set_label(&gettext("_Save"));
            imp.slug_group.set_visible(false);
        } else {
            dialog.set_title(&gettext("New Project"));
            imp.save_button.set_label(&gettext("_Add"));
        }
        imp.shown.replace(Some(shown.clone()));
        imp.taken.replace(
            projects
                .iter()
                .map(|project| (project.slug.clone(), project.name.clone()))
                .collect(),
        );
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
        // None is chosen while the file has a category this version does not
        // know, which stays then.
        let chosen = imp
            .category_buttons
            .borrow()
            .iter()
            .position(gtk::CheckButton::is_active)
            .map(|index| ProjectCategory::ALL[index]);
        if let Some(category) = chosen
            && (category != shown.category() || shown.unknown_category().is_some())
        {
            project.set_category(category);
        }
        let status = PROJECT_STATUSES[imp.status_row.selected() as usize];
        changed(&mut project.status, &shown.status, status);
        Ok(())
    }

    /// Adds a row with a check button for each category, which explains it.
    fn add_category_rows(&self) {
        let imp = self.imp();
        let mut buttons: Vec<gtk::CheckButton> = Vec::new();
        for category in ProjectCategory::ALL {
            let button = gtk::CheckButton::builder()
                .valign(gtk::Align::Center)
                .build();
            button.set_group(buttons.first());
            let row = adw::ActionRow::builder()
                .title(project_category_name(category))
                .subtitle(category_description(category))
                .activatable_widget(&button)
                .build();
            row.add_prefix(&button);
            imp.category_group.add(&row);
            buttons.push(button);
        }
        imp.category_buttons.replace(buttons);
    }

    /// Chooses the category of `project`, or none if this version does not
    /// know it, and says so.
    fn show_category(&self, project: &Project) {
        let imp = self.imp();
        if let Some(unknown) = project.unknown_category() {
            imp.category_group.set_description(Some(
                &gettext("The file has the unknown category “{category}”, which counts as work.")
                    .replace("{category}", unknown),
            ));
            return;
        }
        let index = ProjectCategory::ALL
            .iter()
            .position(|category| *category == project.category())
            .expect("every category is listed");
        imp.category_buttons.borrow()[index].set_active(true);
    }

    /// Shows the repository `repo` of the project on this device, or why
    /// it cannot be read.
    pub fn show_repo(&self, repo: Result<Option<PathBuf>, String>) {
        let imp = self.imp();
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

    /// Allows saving once there is a name and, for a new project, a valid
    /// ID no other project has.
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
                gettext("The project “{name}” already has this ID.").replace("{name}", name)
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

/// What projects of `category` hold, in terms of the two questions behind it.
fn category_description(category: ProjectCategory) -> String {
    match category {
        ProjectCategory::Work => gettext("Work that counts as working time"),
        ProjectCategory::Private => {
            gettext("Work that doesn’t count as working time, such as a side project")
        }
        ProjectCategory::Duty => {
            gettext("Counts as working time, but isn’t work, such as a business trip")
        }
        ProjectCategory::Break => gettext("Time off that doesn’t count"),
    }
}
