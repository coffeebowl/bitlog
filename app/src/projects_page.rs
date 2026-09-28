use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::Local;
use gettextrs::{gettext, ngettext};
use glib::subclass::Signal;
use gtk::glib;
use knotbook_core::{NotePath, Project, ProjectSlug, SaveError, Vault};
use knotbook_index::Found;

use crate::alert::show_error;
use crate::colors::color_dot;
use crate::format::{PROJECT_STATUSES, status_name};
use crate::note_view::NoteView;
use crate::project_dialog::ProjectDialog;
use crate::project_view::ProjectView;
use crate::search_index::SearchIndex;

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/projects_page.ui")]
    pub struct ProjectsPage {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// One group per status that has projects.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        /// Pushed on top of the list, owned here because they are only in
        /// the navigation view while shown.
        pub project_view: ProjectView,
        pub note_view: NoteView,
        /// Where the links to a note lie, before it is renamed.
        pub index: RefCell<SearchIndex>,
        #[template_child]
        pub nav: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub list: TemplateChild<adw::PreferencesPage>,
    }

    impl Default for ProjectsPage {
        fn default() -> Self {
            Self {
                vault: RefCell::default(),
                groups: RefCell::default(),
                project_view: glib::Object::new(),
                note_view: glib::Object::new(),
                index: RefCell::default(),
                nav: TemplateChild::default(),
                stack: TemplateChild::default(),
                list: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectsPage {
        const NAME: &'static str = "KnotbookProjectsPage";
        type Type = super::ProjectsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("projects.add", None, |page, _, _| page.edit(None));
            klass.install_action(
                "projects.edit",
                Some(glib::VariantTy::STRING),
                |page, _, slug| {
                    page.edit(Some(slug_param(slug)));
                },
            );
            klass.install_action(
                "projects.open",
                Some(glib::VariantTy::STRING),
                |page, _, slug| {
                    page.open_project(&slug_param(slug));
                },
            );
            klass.install_action_async(
                "notes.new",
                Some(glib::VariantTy::STRING),
                |page, _, slug| async move { page.new_note(slug_param(slug.as_ref())).await },
            );
            klass.install_action(
                "notes.open",
                Some(glib::VariantTy::STRING),
                |page, _, note| {
                    page.open_note(&note_param(note));
                },
            );
            klass.install_action_async(
                "notes.follow",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.follow_link(note_param(note.as_ref())).await },
            );
            klass.install_action_async(
                "notes.rename",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.rename_note(note_param(note.as_ref())).await },
            );
            klass.install_action_async(
                "notes.delete",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.delete_note(note_param(note.as_ref())).await },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectsPage {
        fn constructed(&self) {
            self.parent_constructed();
            // Back from a note, the previews show it as just typed.
            self.project_view.connect_showing(glib::clone!(
                #[weak(rename_to = note_view)]
                self.note_view,
                move |project_view| {
                    note_view.save_now();
                    project_view.update_previews();
                }
            ));
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted after a project was saved, with the changed
                    // vault available through `vault()`.
                    Signal::builder("vault-changed").build(),
                    // Emitted after links in day files were changed, which
                    // watching the vault leaves out as own writes.
                    Signal::builder("days-changed").build(),
                ]
            })
        }
    }

    impl WidgetImpl for ProjectsPage {}
    impl NavigationPageImpl for ProjectsPage {}
}

glib::wrapper! {
    /// All projects of the vault, to add and edit, and to open one with its
    /// notes.
    pub struct ProjectsPage(ObjectSubclass<imp::ProjectsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn slug_param(param: Option<&glib::Variant>) -> ProjectSlug {
    param
        .and_then(|param| param.str()?.parse().ok())
        .expect("project actions take a slug")
}

fn note_param(param: Option<&glib::Variant>) -> NotePath {
    param
        .and_then(|param| param.str()?.parse().ok())
        .expect("note actions take a note path")
}

impl ProjectsPage {
    /// Shows the projects of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        imp.note_view.set_vault(vault.clone());
        imp.vault.replace(Some(vault));
    }

    /// Uses `index` to find the links to notes.
    pub fn set_index(&self, index: SearchIndex) {
        let imp = self.imp();
        imp.note_view.set_index(index.clone());
        imp.project_view.set_index(index.clone());
        imp.index.replace(index);
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

    pub fn connect_days_changed(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "days-changed",
            false,
            glib::closure_local!(move |page: &Self| callback(page)),
        );
    }

    /// Saves the note being typed, if there are unsaved changes.
    pub fn save_now(&self) {
        self.imp().note_view.save_now();
    }

    /// Shows the projects of the vault, grouped by status, and the project
    /// and note open as they are now.
    pub fn reload(&self) {
        let imp = self.imp();
        for group in imp.groups.take() {
            imp.list.remove(&group);
        }
        let vault = self.vault();
        let mut groups = Vec::new();
        for status in PROJECT_STATUSES {
            let mut projects: Vec<&Project> = vault
                .projects()
                .iter()
                .filter(|project| project.status == status)
                .collect();
            if projects.is_empty() {
                continue;
            }
            projects.sort_by_key(|project| project.name.to_lowercase());
            let group = adw::PreferencesGroup::builder()
                .title(status_name(status))
                .build();
            for project in projects {
                group.add(&project_row(project));
            }
            imp.list.add(&group);
            groups.push(group);
        }
        imp.stack
            .set_visible_child_name(if groups.is_empty() { "empty" } else { "list" });
        imp.groups.replace(groups);

        if self.shows("project") && !self.show_project_again() {
            imp.nav.pop_to_tag("projects");
        }
        if self.shows("note") && !imp.note_view.reload() {
            self.close_note();
        }
    }

    /// Shows the notes `changed`, which were changed elsewhere, as they are
    /// now.
    pub fn notes_changed(&self, changed: &[NotePath]) {
        let imp = self.imp();
        imp.note_view.update_links();
        let slug = imp.project_view.slug();
        if self.shows("project")
            && changed
                .iter()
                .any(|note| Some(note.project()) == slug.as_ref())
        {
            self.show_project_again();
        }
        let shown = imp.note_view.note();
        if self.shows("note")
            && changed.iter().any(|note| Some(note) == shown.as_ref())
            && !imp.note_view.reload()
        {
            self.close_note();
        }
    }

    /// Whether the page with `tag` is in the navigation view.
    fn shows(&self, tag: &str) -> bool {
        self.imp().nav.find_page(tag).is_some()
    }

    /// Shows the project open with its notes as they are now, and marks the
    /// links of the note open again. Returns whether the project is still
    /// there.
    fn show_project_again(&self) -> bool {
        let imp = self.imp();
        imp.note_view.update_links();
        let vault = self.vault();
        let project = imp
            .project_view
            .slug()
            .and_then(|slug| vault.project(&slug).cloned());
        if let Some(project) = &project {
            imp.project_view.show(&vault, project);
        }
        project.is_some()
    }

    /// Shows the note `note` below its project, from where going back leads
    /// to the project's notes.
    pub fn show_note(&self, note: &NotePath) {
        self.open_project(note.project());
        self.open_note(note);
    }

    fn open_project(&self, slug: &ProjectSlug) {
        let imp = self.imp();
        let vault = self.vault();
        let Some(project) = vault.project(slug) else {
            return;
        };
        imp.project_view.show(&vault, project);
        imp.nav.pop_to_tag("projects");
        imp.nav.push(&imp.project_view);
    }

    fn open_note(&self, note: &NotePath) {
        let imp = self.imp();
        // A link may lead to a note of another project, which going back
        // should show.
        if imp.project_view.slug().as_ref() != Some(note.project()) {
            let vault = self.vault();
            if let Some(project) = vault.project(note.project()) {
                imp.project_view.show(&vault, project);
            }
        }
        match imp.note_view.show_note(note) {
            Ok(()) if !self.shows("note") => imp.nav.push(&imp.note_view),
            Ok(()) => {}
            Err(err) => show_error(self, &gettext("Cannot Open Note"), &err.to_string()),
        }
    }

    /// Goes back from the note, which is no longer there.
    fn close_note(&self) {
        let imp = self.imp();
        imp.note_view.forget();
        imp.nav.pop_to_page(&imp.project_view);
    }

    async fn new_note(&self, project: ProjectSlug) {
        let Some(name) = self
            .ask_note_name(&gettext("New Note"), &gettext("_Create"), &project, "")
            .await
        else {
            return;
        };
        let created = self
            .vault()
            .create_note(&project, &name, Local::now().date_naive());
        match created {
            Ok(note) => {
                self.show_project_again();
                self.open_note(&note);
            }
            Err(err) => show_error(self, &gettext("Cannot Create Note"), &err.to_string()),
        }
    }

    /// Opens the note a wiki link points to, or offers to create it.
    async fn follow_link(&self, note: NotePath) {
        let vault = self.vault();
        if vault.note_path(&note).is_file() {
            self.open_note(&note);
            return;
        }
        let project = vault.project_name(note.project());
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Create Note?"))
            .body(
                gettext("There is no note “{name}” in {project} yet.")
                    .replace("{name}", note.name())
                    .replace("{project}", project),
            )
            .close_response("cancel")
            .default_response("create")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("create", &gettext("_Create")),
        ]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        if dialog.choose_future(Some(self)).await != "create" {
            return;
        }
        match vault.create_note(note.project(), note.name(), Local::now().date_naive()) {
            Ok(note) => {
                self.show_project_again();
                self.open_note(&note);
            }
            Err(err) => show_error(self, &gettext("Cannot Create Note"), &err.to_string()),
        }
    }

    async fn rename_note(&self, note: NotePath) {
        let imp = self.imp();
        let Some(name) = self
            .ask_note_name(
                &gettext("Rename Note"),
                &gettext("_Rename"),
                note.project(),
                note.name(),
            )
            .await
        else {
            return;
        };
        // Renaming may change the links in the note shown, and the index
        // has to find what was just typed.
        imp.note_view.save_now();
        let vault = self.vault();
        let index = imp.index.borrow().clone();
        let linking = match index.backlinks(&vault, note.clone()).await {
            Ok(linking) => linking,
            Err(err) => {
                show_error(self, &gettext("Cannot Rename Note"), &err.to_string());
                return;
            }
        };
        let itself = Found::Note(note.clone());
        let others = linking.iter().filter(|link| link.found != itself).count();
        let update_links = if others == 0 {
            // Only the note itself links to it, if at all.
            true
        } else {
            match self.ask_update_links(&note, others).await {
                Some(update) => update,
                None => return,
            }
        };
        match vault.rename_note(&note, &name, update_links) {
            Ok(renamed) => {
                if update_links {
                    self.emit_by_name::<()>("days-changed", &[]);
                }
                self.show_project_again();
                if imp.note_view.note() == Some(note) {
                    imp.note_view.forget();
                    self.open_note(&renamed);
                } else {
                    imp.note_view.reload();
                }
            }
            Err(err) => show_error(self, &gettext("Cannot Rename Note"), &err.to_string()),
        }
    }

    async fn delete_note(&self, note: NotePath) {
        let imp = self.imp();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Delete Note?"))
            .body(gettext("“{name}” will be deleted for good.").replace("{name}", note.name()))
            .close_response("cancel")
            .default_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("delete", &gettext("_Delete")),
        ]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        if dialog.choose_future(Some(self)).await != "delete" {
            return;
        }
        let shown = imp.note_view.note() == Some(note.clone());
        if shown {
            self.close_note();
        }
        if let Err(err) = self.vault().delete_note(&note) {
            show_error(self, &gettext("Cannot Delete Note"), &err.to_string());
        }
        self.show_project_again();
    }

    /// Asks for the name of a note in `project`, starting with `name`.
    /// Returns `None` if the user cancels.
    async fn ask_note_name(
        &self,
        heading: &str,
        accept: &str,
        project: &ProjectSlug,
        name: &str,
    ) -> Option<String> {
        let entry = gtk::Entry::builder()
            .text(name)
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .extra_child(&entry)
            .close_response("cancel")
            .default_response("accept")
            .focus_widget(&entry)
            .build();
        dialog.add_responses(&[("cancel", &gettext("_Cancel")), ("accept", accept)]);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        let is_new_name = {
            let (project, name) = (project.clone(), name.to_owned());
            move |text: &str| {
                let text = text.trim();
                text != name && NotePath::new(project.clone(), text).is_ok()
            }
        };
        dialog.set_response_enabled("accept", false);
        entry.connect_changed(glib::clone!(
            #[weak]
            dialog,
            move |entry| dialog.set_response_enabled("accept", is_new_name(&entry.text()))
        ));
        let response = dialog.choose_future(Some(self)).await;
        (response == "accept").then(|| entry.text().trim().to_owned())
    }

    /// Asks whether the links in `others` other notes, day notes and blocks
    /// to `note` should point to its new name. Returns `None` if the user
    /// cancels.
    async fn ask_update_links(&self, note: &NotePath, others: usize) -> Option<bool> {
        let count = u32::try_from(others).unwrap_or(u32::MAX);
        let body = ngettext(
            "{count} other place links to “{name}”. Should the link point to the new name?",
            "{count} other places link to “{name}”. Should the links point to the new name?",
            count,
        )
        .replace("{count}", &others.to_string())
        .replace("{name}", note.name());
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Update Links?"))
            .body(body)
            .close_response("cancel")
            .default_response("update")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("keep", &gettext("_Keep Links")),
            ("update", &gettext("_Update Links")),
        ]);
        dialog.set_response_appearance("update", adw::ResponseAppearance::Suggested);
        match dialog.choose_future(Some(self)).await.as_str() {
            "update" => Some(true),
            "keep" => Some(false),
            _ => None,
        }
    }

    /// Asks for the details of the project `slug`, or of a new project.
    fn edit(&self, slug: Option<ProjectSlug>) {
        let vault = self.vault();
        let project = slug.as_ref().and_then(|slug| vault.project(slug));
        let dialog = ProjectDialog::new(project);
        // Read when the dialog opens, so that saving only writes a change.
        let repo = slug.as_ref().map(|slug| {
            vault
                .repo_paths()
                .map(|mut repos| repos.remove(slug))
                .map_err(|err| err.to_string())
        });
        if let Some(repo) = repo.clone() {
            dialog.show_repo(repo);
        }
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |dialog| page.save(dialog, slug.as_ref(), repo.as_ref())
        ));
        dialog.present(Some(self));
    }

    /// Saves what `dialog` holds for the project `slug`, or as a new
    /// project, and closes it. On failure it stays open. `repo` is the
    /// repository shown when the dialog opened, if it showed one.
    fn save(
        &self,
        dialog: &ProjectDialog,
        slug: Option<&ProjectSlug>,
        repo: Option<&Result<Option<PathBuf>, String>>,
    ) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        let saved: Result<(), SaveError> = match slug {
            Some(slug) => match repo {
                Some(Ok(repo)) if *repo != dialog.repo() => {
                    vault.set_repo_path(slug, dialog.repo().as_deref())
                }
                _ => Ok(()),
            }
            .and_then(|()| vault.update_project(slug, |project| dialog.apply(project)))
            .map(|_| ()),
            None => {
                let slug = dialog
                    .slug()
                    .expect("new projects are saved with a valid ID");
                let mut project = Project::new(slug, &dialog.name(), Local::now().date_naive());
                dialog
                    .apply(&mut project)
                    .map_err(SaveError::from)
                    .and_then(|()| vault.add_project(project).map(|_| ()))
            }
        };
        match saved {
            Ok(()) => {
                dialog.close();
                self.set_vault(Rc::new(vault));
                self.reload();
                self.emit_by_name::<()>("vault-changed", &[]);
            }
            Err(err) => show_error(dialog, &gettext("Cannot Save Project"), &err.to_string()),
        }
    }
}

fn project_row(project: &Project) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&project.name)
        .subtitle(&project.category)
        .use_markup(false)
        .activatable(true)
        .action_name("projects.open")
        .action_target(&project.slug.to_string().to_variant())
        .build();
    let dot = gtk::Label::builder()
        .label(color_dot(&project.color))
        .use_markup(true)
        .build();
    row.add_prefix(&dot);
    if project.pinned {
        let pin = gtk::Image::builder()
            .icon_name("view-pin-symbolic")
            .tooltip_text(gettext("Pinned"))
            .build();
        row.add_suffix(&pin);
    }
    row
}
