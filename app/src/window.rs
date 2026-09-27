use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use knotbook_core::{
    BlockId, ConflictCopy, NotePath, ReadError, TaskId, Vault, VaultChange, VaultWatcher,
    WatchError,
};
use knotbook_index::Found;

use crate::calendar_view::CalendarView;
use crate::config;
use crate::day_view::DayView;
use crate::preferences_dialog::PreferencesDialog;
use crate::projects_page::ProjectsPage;
use crate::reports_page::ReportsPage;
use crate::search_dialog::SearchDialog;
use crate::search_index::SearchIndex;
use crate::sync_conflict_dialog::{SyncConflictDialog, file_title};
use crate::tasks_page::TasksPage;

/// Actions that need an open vault.
const VAULT_ACTIONS: [&str; 17] = [
    "win.preferences",
    "win.previous",
    "win.next",
    "win.today",
    "win.show-today",
    "win.show-calendar",
    "win.show-tasks",
    "win.show-projects",
    "win.show-reports",
    "win.show-day",
    "win.show-block",
    "win.show-note",
    "win.show-task",
    "win.new-block",
    "win.new-project",
    "win.search",
    "win.resolve-conflict",
];

/// The window action that shows `found`, with its target, as the actions
/// below read it.
pub fn show_action(found: &Found) -> (&'static str, glib::Variant) {
    match found {
        Found::Block { date, id } => (
            "win.show-block",
            (date.to_string(), id.to_string()).to_variant(),
        ),
        Found::DayNote(date) => ("win.show-day", date.to_string().to_variant()),
        Found::Note(note) => ("win.show-note", note.to_string().to_variant()),
        Found::Task(id) => ("win.show-task", id.to_string().to_variant()),
    }
}

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/window.ui")]
    pub struct Window {
        pub settings: gio::Settings,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub conflict_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub sidebar_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub today_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub calendar_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub tasks_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub projects_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub reports_row: TemplateChild<gtk::ListBoxRow>,
        /// The pages of the content, owned here because only one of them is
        /// in the split view at a time.
        pub day_view: DayView,
        pub calendar_view: CalendarView,
        pub tasks_page: TasksPage,
        pub projects_page: ProjectsPage,
        pub reports_page: ReportsPage,
        /// The folder of the open vault.
        pub vault_path: RefCell<Option<PathBuf>>,
        /// The open vault as the pages have it.
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub watcher: RefCell<Option<VaultWatcher>>,
        pub index: RefCell<SearchIndex>,
        /// The sync conflict copies that need a decision.
        pub conflicts: RefCell<Vec<ConflictCopy>>,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::app_id()),
                toast_overlay: TemplateChild::default(),
                stack: TemplateChild::default(),
                split_view: TemplateChild::default(),
                sidebar: TemplateChild::default(),
                conflict_banner: TemplateChild::default(),
                sidebar_list: TemplateChild::default(),
                today_row: TemplateChild::default(),
                calendar_row: TemplateChild::default(),
                tasks_row: TemplateChild::default(),
                projects_row: TemplateChild::default(),
                reports_row: TemplateChild::default(),
                day_view: glib::Object::new(),
                calendar_view: glib::Object::new(),
                tasks_page: glib::Object::new(),
                projects_page: glib::Object::new(),
                reports_page: glib::Object::new(),
                vault_path: RefCell::default(),
                vault: RefCell::default(),
                watcher: RefCell::default(),
                index: RefCell::default(),
                conflicts: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "KnotbookWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action_async("win.open-vault", None, |window, _, _| async move {
                window.choose_vault().await;
            });
            klass.install_action_async("win.new-vault", None, |window, _, _| async move {
                window.create_vault().await;
            });
            // Previous, next and today move by the unit of the page shown,
            // and do nothing on the task and project pages.
            klass.install_action("win.show-reports", None, |window, _, _| {
                window.show_reports();
            });
            klass.install_action("win.resolve-conflict", None, |window, _, _| {
                window.resolve_conflict();
            });
            klass.install_action("win.previous", None, |window, _, _| window.step(-1));
            klass.install_action("win.next", None, |window, _, _| window.step(1));
            klass.install_action("win.today", None, |window, _, _| {
                let today = Local::now().date_naive();
                let imp = window.imp();
                if window.shows_calendar() {
                    imp.calendar_view.show(today);
                } else if window.shows_reports() {
                    imp.reports_page.show(today);
                } else if window.shows_day() {
                    imp.day_view.show_date(today);
                }
            });
            klass.install_action("win.show-today", None, |window, _, _| {
                window.show_day(Local::now().date_naive());
            });
            // Forwarded, so that the shortcut works wherever the focus is.
            klass.install_action("win.new-block", None, |window, _, _| {
                if window.shows_day() {
                    // Fails on days without a file, which have no blocks yet.
                    let _ =
                        WidgetExt::activate_action(&window.imp().day_view, "day.new-block", None);
                }
            });
            klass.install_action("win.show-calendar", None, |window, _, _| {
                window.show_calendar();
            });
            klass.install_action("win.show-tasks", None, |window, _, _| {
                window.show_tasks();
            });
            klass.install_action("win.show-projects", None, |window, _, _| {
                window.show_projects();
            });
            klass.install_action("win.new-project", None, |window, _, _| {
                window.show_projects();
                WidgetExt::activate_action(&window.imp().projects_page, "projects.add", None)
                    .expect("the project page adds projects");
            });
            klass.install_action("win.search", None, |window, _, _| window.search());
            klass.install_action("win.preferences", None, |window, _, _| {
                window.show_preferences();
            });
            klass.install_action(
                "win.show-block",
                Some(glib::VariantTy::new("(ss)").expect("(ss) is a variant type")),
                |window, _, block| {
                    let (date, id): (String, String) = block
                        .and_then(|block| block.get())
                        .expect("blocks are passed as date and id");
                    let date = date.parse().expect("dates are passed as YYYY-MM-DD");
                    let id: BlockId = id.parse().expect("block ids are passed as they are");
                    window.show_day(date);
                    window.imp().day_view.show_block_by_id(&id);
                },
            );
            klass.install_action(
                "win.show-note",
                Some(glib::VariantTy::STRING),
                |window, _, note| {
                    let note: NotePath = note
                        .and_then(|note| note.str()?.parse().ok())
                        .expect("notes are passed as their path");
                    window.show_projects();
                    window.imp().projects_page.show_note(&note);
                },
            );
            klass.install_action(
                "win.show-task",
                Some(glib::VariantTy::STRING),
                |window, _, id| {
                    let id: TaskId = id
                        .and_then(|id| id.str()?.parse().ok())
                        .expect("tasks are passed as their id");
                    window.show_tasks();
                    window.imp().tasks_page.show_task(&id);
                },
            );
            klass.install_action(
                "win.show-day",
                Some(glib::VariantTy::STRING),
                |window, _, date| {
                    let date = date
                        .and_then(|date| date.str()?.parse().ok())
                        .expect("the calendar passes dates as YYYY-MM-DD");
                    window.show_day(date);
                },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            self.split_view.set_content(Some(&self.day_view));
            self.projects_page.connect_vault_changed(glib::clone!(
                #[weak(rename_to = window)]
                self.obj(),
                move |page| window.projects_changed(page.vault())
            ));
            self.obj().connect_close_request(|window| {
                window.save_texts_now();
                glib::Propagation::Proceed
            });
            for action in VAULT_ACTIONS {
                self.obj().action_set_enabled(action, false);
            }
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    pub fn new(app: &adw::Application) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        window.remember_size();
        window.open_last_vault();
        window
    }

    /// Keeps the window size in GSettings, so that the next window starts with it.
    fn remember_size(&self) {
        let settings = &self.imp().settings;
        settings.bind("window-width", self, "default-width").build();
        settings
            .bind("window-height", self, "default-height")
            .build();
        settings
            .bind("window-is-maximized", self, "maximized")
            .build();
    }

    /// Opens the vault of the last session, if there is one.
    fn open_last_vault(&self) {
        let uri = self.imp().settings.string("last-vault");
        if uri.is_empty() {
            return;
        }
        let folder = gio::File::for_uri(&uri);
        match folder.path() {
            Some(path) => self.open_vault(&path),
            None => self.show_error(
                &gettext("Cannot Open Vault"),
                &gettext("The vault is not on a local file system."),
            ),
        }
    }

    async fn choose_vault(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Vault"))
            .modal(true)
            .build();
        // Dismissing the dialog is reported as an error, too.
        let Ok(folder) = dialog.select_folder_future(Some(self)).await else {
            return;
        };
        match folder.path() {
            Some(path) => self.open_vault(&path),
            None => self.show_error(
                &gettext("Cannot Open Vault"),
                &gettext("Choose a folder on a local file system."),
            ),
        }
    }

    /// Creates a vault in a folder the user chooses and opens it.
    async fn create_vault(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("New Vault"))
            .accept_label(gettext("_Create"))
            .modal(true)
            .build();
        // Dismissing the dialog is reported as an error, too.
        let Ok(folder) = dialog.select_folder_future(Some(self)).await else {
            return;
        };
        let Some(path) = folder.path() else {
            self.show_error(
                &gettext("Cannot Create Vault"),
                &gettext("Choose a folder on a local file system."),
            );
            return;
        };
        self.create_vault_in(&path);
    }

    /// Creates a vault in `path` and opens it. Like the CLI, it is named
    /// after its folder.
    fn create_vault_in(&self, path: &Path) {
        let name = path
            .file_name()
            .map_or("Knotbook".into(), |name| name.to_string_lossy());
        match Vault::create(path, &name, Local::now().date_naive()) {
            Ok(_) => self.open_vault(path),
            Err(err) => self.show_error(&gettext("Cannot Create Vault"), &err.to_string()),
        }
    }

    /// Opens the vault in `path` and remembers it for the next start.
    ///
    /// On failure the window keeps showing what it showed before.
    fn open_vault(&self, path: &Path) {
        let imp = self.imp();
        if let Err(err) = self.load_vault(path) {
            self.show_error(&gettext("Cannot Open Vault"), &err.to_string());
            return;
        }
        let today = Local::now().date_naive();
        imp.calendar_view.show(today);
        imp.day_view.show_date(today);
        imp.split_view.set_content(Some(&imp.day_view));
        imp.sidebar_list.select_row(Some(&*imp.today_row));
        imp.stack.set_visible_child_name("vault");
        for action in VAULT_ACTIONS {
            self.action_set_enabled(action, true);
        }
        self.check_conflicts();
        imp.settings
            .set_string("last-vault", &gio::File::for_path(path).uri())
            .expect("the last vault can be stored");
    }

    /// Reads the vault in `path` for all pages and watches it for changes
    /// made elsewhere. On failure everything stays as it was.
    fn load_vault(&self, path: &Path) -> Result<(), ReadError> {
        let imp = self.imp();
        let vault = Rc::new(Vault::open(path)?);
        self.set_vault(&vault);
        imp.vault_path.replace(Some(path.to_owned()));
        // Built in the background, so that the first search is quick.
        let index = SearchIndex::default();
        imp.index.replace(index.clone());
        imp.projects_page.set_index(index.clone());
        imp.reports_page.set_index(index.clone());
        let indexed = vault.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = index.update(&indexed).await {
                glib::g_warning!("knotbook", "{err}");
            }
        });
        // The watcher tells its own writes apart through the vault it
        // belongs to, so every vault gets a new one.
        let window: glib::SendWeakRef<Self> = self.downgrade().into();
        let watcher = vault.watch(move |changes| {
            let window = window.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(window) = window.upgrade() {
                    window.vault_changed(changes);
                }
            });
        });
        match watcher {
            Ok(watcher) => {
                imp.watcher.replace(Some(watcher));
            }
            Err(err) => {
                imp.watcher.replace(None);
                self.show_watch_error(&err);
            }
        }
        Ok(())
    }

    fn set_vault(&self, vault: &Rc<Vault>) {
        let imp = self.imp();
        imp.vault.replace(Some(vault.clone()));
        imp.sidebar.set_title(&vault.config().name);
        imp.calendar_view.set_vault(vault.clone());
        imp.day_view.set_vault(vault.clone());
        imp.tasks_page.set_vault(vault.clone());
        imp.projects_page.set_vault(vault.clone());
        imp.reports_page.set_vault(vault.clone());
    }

    /// Hands `vault`, with projects changed on the project page, to the
    /// other pages. It shares the record of own writes with the vault
    /// watched, so the watcher stays.
    fn projects_changed(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        self.set_vault(&vault);
        imp.day_view.show_date(imp.day_view.date());
    }

    /// Shows what was changed elsewhere, by sync or the CLI.
    fn vault_changed(&self, changes: Result<Vec<VaultChange>, WatchError>) {
        match changes {
            Ok(changes) => {
                self.show_changes(&changes);
                self.check_conflicts();
            }
            // Watching goes on, a later change may be seen again.
            Err(err) => glib::g_warning!("knotbook", "{err}"),
        }
    }

    /// Merges the sync conflict copies without contradictions, and offers
    /// to resolve the others.
    fn check_conflicts(&self) {
        let imp = self.imp();
        let vault = imp
            .vault
            .borrow()
            .clone()
            .expect("conflicts are checked in an open vault");
        let copies = match vault.conflict_copies() {
            Ok(copies) => copies,
            Err(err) => {
                glib::g_warning!("knotbook", "{err}");
                Vec::new()
            }
        };
        let mut open = Vec::new();
        for copy in copies {
            let merged = vault.contradictions(&copy).map(|contradictions| {
                contradictions.is_empty() && vault.merge_conflict(&copy, &[]).is_ok()
            });
            match merged {
                Ok(true) => {
                    let toast = gettext("Merged a sync conflict of {file}")
                        .replace("{file}", &file_title(&vault, &copy.of));
                    imp.toast_overlay.add_toast(adw::Toast::new(&toast));
                    // Own writes are not watched.
                    self.show_changes(std::slice::from_ref(&copy.of));
                }
                Ok(false) => open.push(copy),
                // An unreadable copy cannot be resolved here; `knotbook
                // doctor` names it.
                Err(err) => glib::g_warning!("knotbook", "{err}"),
            }
        }
        let count = u32::try_from(open.len()).unwrap_or(u32::MAX);
        imp.conflict_banner.set_title(
            &ngettext("{count} sync conflict", "{count} sync conflicts", count)
                .replace("{count}", &count.to_string()),
        );
        imp.conflict_banner.set_revealed(!open.is_empty());
        imp.conflicts.replace(open);
    }

    /// Lets the user resolve the first sync conflict that needs a decision.
    fn resolve_conflict(&self) {
        let imp = self.imp();
        let Some(copy) = imp.conflicts.borrow().first().cloned() else {
            return;
        };
        self.save_texts_now();
        let vault = imp.vault.borrow().clone().expect("conflicts need a vault");
        match SyncConflictDialog::new(vault, copy.clone()) {
            Ok(dialog) => {
                dialog.connect_merged(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || {
                        // Own writes are not watched.
                        window.show_changes(std::slice::from_ref(&copy.of));
                        window.check_conflicts();
                    }
                ));
                dialog.present(Some(self));
            }
            Err(err) => self.show_error(&gettext("Cannot Read Sync Conflict"), &err.to_string()),
        }
    }

    /// Shows the files of `changes` as they are now.
    fn show_changes(&self, changes: &[VaultChange]) {
        let imp = self.imp();
        let changes_vault =
            |change: &VaultChange| matches!(change, VaultChange::Config | VaultChange::Project(_));
        if changes.iter().any(changes_vault) {
            self.reload_vault();
            return;
        }
        if changes.contains(&VaultChange::Day(imp.day_view.date())) {
            imp.day_view.reload();
        }
        if changes.contains(&VaultChange::Tasks) {
            imp.day_view.show_tasks();
            if self.shows_tasks() {
                imp.tasks_page.reload();
            }
        }
        let notes: Vec<NotePath> = changes
            .iter()
            .filter_map(|change| match change {
                VaultChange::Note(note) => Some(note.clone()),
                _ => None,
            })
            .collect();
        if !notes.is_empty() {
            imp.projects_page.notes_changed(&notes);
        }
        let changes_day = |change: &VaultChange| matches!(change, VaultChange::Day(_));
        if self.shows_calendar() && changes.iter().any(changes_day) {
            imp.calendar_view.reload();
        }
        if self.shows_reports() && changes.iter().any(changes_day) {
            imp.reports_page.reload();
        }
    }

    /// Reads settings and projects again, keeping the pages where they are.
    fn reload_vault(&self) {
        let imp = self.imp();
        let path = imp
            .vault_path
            .borrow()
            .clone()
            .expect("changes are only watched in an open vault");
        let date = imp.day_view.date();
        match self.load_vault(&path) {
            Ok(()) => {
                imp.day_view.show_date(date);
                imp.calendar_view.reload();
                imp.tasks_page.reload();
                imp.projects_page.reload();
                if self.shows_reports() {
                    imp.reports_page.reload();
                }
                self.check_conflicts();
            }
            Err(err) => self.show_error(&gettext("Cannot Open Vault"), &err.to_string()),
        }
    }

    fn shows_day(&self) -> bool {
        let imp = self.imp();
        imp.split_view.content().as_ref() == Some(imp.day_view.upcast_ref())
    }

    fn shows_calendar(&self) -> bool {
        let imp = self.imp();
        imp.split_view.content().as_ref() == Some(imp.calendar_view.upcast_ref())
    }

    fn shows_reports(&self) -> bool {
        let imp = self.imp();
        imp.split_view.content().as_ref() == Some(imp.reports_page.upcast_ref())
    }

    fn shows_tasks(&self) -> bool {
        let imp = self.imp();
        imp.split_view.content().as_ref() == Some(imp.tasks_page.upcast_ref())
    }

    /// Shows the day, week or month `steps` steps after the one shown now.
    fn step(&self, steps: i32) {
        let imp = self.imp();
        if self.shows_calendar() {
            imp.calendar_view.step(steps);
        } else if self.shows_reports() {
            imp.reports_page.step(steps);
        } else if self.shows_day() {
            let date = imp.day_view.date();
            let date = date.checked_add_signed(TimeDelta::days(steps.into()));
            imp.day_view
                .show_date(date.expect("nobody steps this way to the end of the calendar"));
        }
    }

    fn show_day(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.projects_page.save_now();
        imp.day_view.show_date(date);
        imp.split_view.set_content(Some(&imp.day_view));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.today_row));
    }

    fn show_calendar(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.calendar_view.reload();
        imp.split_view.set_content(Some(&imp.calendar_view));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.calendar_row));
    }

    fn show_tasks(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.tasks_page.reload();
        imp.split_view.set_content(Some(&imp.tasks_page));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.tasks_row));
    }

    fn show_projects(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.projects_page.reload();
        imp.split_view.set_content(Some(&imp.projects_page));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.projects_row));
    }

    fn show_preferences(&self) {
        let imp = self.imp();
        if self.visible_dialog().is_some() {
            return;
        }
        let vault = imp
            .vault
            .borrow()
            .clone()
            .expect("preferences need a vault");
        let dialog = PreferencesDialog::new(&vault);
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog| window.save_preferences(dialog)
        ));
        dialog.present(Some(self));
    }

    /// Saves the settings `dialog` holds and closes it. On failure it
    /// stays open.
    fn save_preferences(&self, dialog: &PreferencesDialog) {
        let imp = self.imp();
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(
            &imp.vault
                .borrow()
                .clone()
                .expect("preferences need a vault"),
        );
        match vault.update_config(|config| dialog.apply(config)) {
            Ok(_) => {
                dialog.close();
                self.config_changed(Rc::new(vault));
            }
            Err(err) => {
                let alert = adw::AlertDialog::new(
                    Some(&gettext("Cannot Save Preferences")),
                    Some(&err.to_string()),
                );
                alert.add_response("close", &gettext("_Close"));
                alert.present(Some(dialog));
            }
        }
    }

    /// Hands `vault`, with changed settings, to all pages and shows them
    /// anew. Blocks keep their times.
    fn config_changed(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        self.set_vault(&vault);
        imp.day_view.show_date(imp.day_view.date());
        imp.calendar_view.reload();
        imp.projects_page.reload();
        if self.shows_reports() {
            imp.reports_page.reload();
        }
    }

    /// Opens the search, which also offers commands.
    fn search(&self) {
        let imp = self.imp();
        // Ctrl+K while it is open.
        if self.visible_dialog().is_some() {
            return;
        }
        // So that the search finds what was just typed.
        self.save_texts_now();
        let vault = imp.vault.borrow().clone().expect("search needs a vault");
        let dialog = SearchDialog::new(vault, imp.index.borrow().clone(), self.shows_day());
        dialog.present(Some(self));
    }

    /// Shows the reports of the period shown before, at first the current
    /// month.
    fn show_reports(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.reports_page.reload();
        imp.split_view.set_content(Some(&imp.reports_page));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.reports_row));
    }

    /// Saves what is being typed on any page, if there are unsaved changes.
    fn save_texts_now(&self) {
        let imp = self.imp();
        imp.day_view.save_texts_now();
        imp.projects_page.save_now();
    }

    fn show_watch_error(&self, err: &WatchError) {
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Cannot Watch Vault")),
            Some(&format!(
                "{}\n\n{err}",
                gettext("Changes made elsewhere only show up after going to another day.")
            )),
        );
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
    }

    fn show_error(&self, heading: &str, message: &str) {
        let dialog = adw::AlertDialog::new(Some(heading), Some(message));
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
    }
}
