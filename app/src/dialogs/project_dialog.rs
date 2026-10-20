use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{
    EditError, Project, ProjectCategory, ProjectSlug, ProjectStatus, check_repo_path, repo_name,
};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::colors;
use crate::format::{project_category_name, project_status_name};
use crate::widgets::{changed, remove_button, set_class};

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
        /// The repository folders as chosen.
        pub repos: RefCell<Vec<PathBuf>>,
        /// The rows in `repo_group` that show `repos`, in the same order.
        pub repo_rows: RefCell<Vec<adw::ActionRow>>,
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
        pub repo_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub add_repo_button: TemplateChild<gtk::Button>,
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
            self.add_repo_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    glib::spawn_future_local(async move { dialog.choose_repo(None).await });
                }
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
        dialog.show_status(shown);
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
        // The status of the file this version does not know, listed last,
        // stays unless another one is chosen.
        let chosen = ProjectStatus::ALL.get(imp.status_row.selected() as usize);
        if let Some(&status) = chosen
            && (status != shown.status() || shown.unknown_status().is_some())
        {
            project.set_status(status);
        }
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

    /// Lists the statuses with that of `project` chosen, and after them a
    /// status of the file this version does not know, which it explains.
    fn show_status(&self, project: &Project) {
        let imp = self.imp();
        let mut names: Vec<String> = ProjectStatus::ALL
            .into_iter()
            .map(project_status_name)
            .collect();
        let chosen = match project.unknown_status() {
            Some(unknown) => {
                imp.status_row.set_subtitle(
                    &gettext("The file has the unknown status “{status}”, which counts as active.")
                        .replace("{status}", unknown),
                );
                names.push(unknown.to_owned());
                names.len() - 1
            }
            None => ProjectStatus::ALL
                .iter()
                .position(|status| *status == project.status())
                .expect("every status is listed"),
        };
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        imp.status_row
            .set_model(Some(&gtk::StringList::new(&names)));
        imp.status_row
            .set_selected(u32::try_from(chosen).expect("few statuses"));
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

    /// Shows the repositories `repos` of the project on this device, or why
    /// they cannot be read.
    pub fn show_repos(&self, repos: Result<Vec<PathBuf>, String>) {
        let imp = self.imp();
        match repos {
            Ok(repos) => repos.into_iter().for_each(|repo| self.add_repo(repo)),
            Err(err) => {
                imp.repo_group.set_description(Some(&err));
                imp.add_repo_button.set_sensitive(false);
            }
        }
    }

    /// The repository folders as chosen.
    pub fn repos(&self) -> Vec<PathBuf> {
        self.imp().repos.borrow().clone()
    }

    /// Adds `repo` to the list with a row that shows it, and with buttons
    /// to choose another folder and to remove it.
    fn add_repo(&self, repo: PathBuf) {
        let imp = self.imp();
        let row = adw::ActionRow::builder().subtitle_selectable(true).build();
        show_repo(&row, &repo);
        let choose_button = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(gettext("Choose Folder"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        choose_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            #[weak]
            row,
            move |_| {
                glib::spawn_future_local(async move { dialog.choose_repo(Some(row)).await });
            }
        ));
        row.add_suffix(&choose_button);
        row.add_suffix(&remove_button(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            #[weak]
            row,
            move || dialog.remove_repo(&row)
        )));
        imp.repo_group.add(&row);
        imp.repos.borrow_mut().push(repo);
        imp.repo_rows.borrow_mut().push(row);
    }

    fn remove_repo(&self, row: &adw::ActionRow) {
        let imp = self.imp();
        let mut rows = imp.repo_rows.borrow_mut();
        let index = rows
            .iter()
            .position(|other| other == row)
            .expect("the row shows a repository");
        rows.remove(index);
        imp.repos.borrow_mut().remove(index);
        imp.repo_group.remove(row);
    }

    /// Asks for a repository folder to put in place of the one `row` shows,
    /// or to add if there is no row.
    async fn choose_repo(&self, row: Option<adw::ActionRow>) {
        let imp = self.imp();
        let index = row.and_then(|row| imp.repo_rows.borrow().iter().position(|r| *r == row));
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose Repository"))
            .modal(true)
            .build();
        // Repositories of a project often lie next to each other. A folder
        // moved away starts at what is left of its path.
        let near = match index {
            Some(index) => imp.repos.borrow().get(index).cloned(),
            None => imp.repos.borrow().last().cloned(),
        };
        if let Some(folder) = near
            .as_deref()
            .and_then(|near| near.ancestors().skip(1).find(|folder| folder.is_dir()))
        {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
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
            Ok(path) if imp.repos.borrow().contains(&path) => {}
            Ok(path) => match index {
                Some(index) => {
                    show_repo(&imp.repo_rows.borrow()[index], &path);
                    imp.repos.borrow_mut()[index] = path;
                }
                None => self.add_repo(path),
            },
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

/// Shows `repo` in `row`, with a warning if its folder holds no repository
/// anymore, as after moving it.
fn show_repo(row: &adw::ActionRow, repo: &Path) {
    row.set_title(&glib::markup_escape_text(&repo_name(repo)));
    let path = repo.display().to_string();
    let missing = check_repo_path(repo).is_err();
    let subtitle = match missing {
        true => gettext("No Git repository at {path}").replace("{path}", &path),
        false => path,
    };
    row.set_subtitle(&glib::markup_escape_text(&subtitle));
    set_class(row, "warning", missing);
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
