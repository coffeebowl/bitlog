mod vault;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{BlockId, NotePath, ProjectSlug, ProjectStatus, TaskId, Vault, VaultWatcher};
use bitlog_index::Found;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::calendar_view::CalendarView;
use crate::colors::color_dot;
use crate::config;
use crate::day_view::DayView;
use crate::notes_page::NotesPage;
use crate::preferences_dialog::PreferencesDialog;
use crate::projects_page::ProjectsPage;
use crate::reports_page::ReportsPage;
use crate::search_dialog::SearchDialog;
use crate::search_index::SearchIndex;
use crate::style_switcher;
use crate::tasks_page::TasksPage;
use crate::vault_check_dialog::VaultCheckDialog;
use crate::widgets::param;

/// Actions that need an open vault.
const VAULT_ACTIONS: [&str; 20] = [
    "win.preferences",
    "win.check-vault",
    "win.previous",
    "win.next",
    "win.today",
    "win.show-today",
    "win.show-calendar",
    "win.show-tasks",
    "win.show-notes",
    "win.show-projects",
    "win.show-project",
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
    #[template(resource = "/dev/bitlog/BitLog/window.ui")]
    pub struct Window {
        pub settings: gio::Settings,
        #[template_child]
        pub welcome_menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub vault_menu_button: TemplateChild<gtk::MenuButton>,
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
        pub notes_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub projects_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub reports_row: TemplateChild<gtk::ListBoxRow>,
        /// The active projects below the pages, each with its row.
        pub project_rows: RefCell<Vec<(ProjectSlug, gtk::ListBoxRow)>>,
        /// The pages of the content, owned here because only one of them is
        /// in the split view at a time.
        pub day_view: DayView,
        pub calendar_view: CalendarView,
        pub tasks_page: TasksPage,
        pub notes_page: NotesPage,
        pub projects_page: ProjectsPage,
        pub reports_page: ReportsPage,
        /// The folder of the open vault.
        pub vault_path: RefCell<Option<PathBuf>>,
        /// The open vault as the pages have it.
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub watcher: RefCell<Option<VaultWatcher>>,
        pub index: RefCell<SearchIndex>,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::app_id()),
                welcome_menu_button: TemplateChild::default(),
                vault_menu_button: TemplateChild::default(),
                toast_overlay: TemplateChild::default(),
                stack: TemplateChild::default(),
                split_view: TemplateChild::default(),
                sidebar: TemplateChild::default(),
                conflict_banner: TemplateChild::default(),
                sidebar_list: TemplateChild::default(),
                today_row: TemplateChild::default(),
                calendar_row: TemplateChild::default(),
                tasks_row: TemplateChild::default(),
                notes_row: TemplateChild::default(),
                projects_row: TemplateChild::default(),
                reports_row: TemplateChild::default(),
                project_rows: RefCell::default(),
                day_view: glib::Object::new(),
                calendar_view: glib::Object::new(),
                tasks_page: glib::Object::new(),
                notes_page: glib::Object::new(),
                projects_page: glib::Object::new(),
                reports_page: glib::Object::new(),
                vault_path: RefCell::default(),
                vault: RefCell::default(),
                watcher: RefCell::default(),
                index: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "BitLogWindow";
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
            klass.install_action("win.show-reports", None, |window, _, _| {
                window.show_reports();
            });
            klass.install_action(
                "win.resolve-conflict",
                Some(glib::VariantTy::STRING),
                |window, _, copy| {
                    let copy: PathBuf = param(copy, "sync conflict copies");
                    window.resolve_conflict(&copy);
                },
            );
            klass.install_action("win.previous", None, |window, _, _| window.step(-1));
            klass.install_action("win.next", None, |window, _, _| window.step(1));
            klass.install_action("win.today", None, |window, _, _| window.step_to_today());
            klass.install_action("win.show-today", None, |window, _, _| {
                window.show_day(Local::now().date_naive());
            });
            klass.install_action("win.new-block", None, |window, _, _| window.new_block());
            klass.install_action("win.show-calendar", None, |window, _, _| {
                window.show_calendar();
            });
            klass.install_action("win.show-tasks", None, |window, _, _| {
                window.show_tasks();
            });
            klass.install_action("win.show-notes", None, |window, _, _| {
                window.show_notes();
                window.imp().notes_page.show_overview();
            });
            klass.install_action("win.show-projects", None, |window, _, _| {
                // First, so that reloading does not build the project left
                // once more.
                window.imp().projects_page.show_overview();
                window.show_projects();
                window.select_project_row();
            });
            klass.install_action(
                "win.show-project",
                Some(glib::VariantTy::STRING),
                |window, _, slug| window.show_project(&param(slug, "projects")),
            );
            klass.install_action("win.new-project", None, |window, _, _| {
                window.show_projects();
                WidgetExt::activate_action(&window.imp().projects_page, "projects.add", None)
                    .expect("the project page adds projects");
            });
            klass.install_action("win.search", None, |window, _, _| window.search());
            klass.install_action("win.preferences", None, |window, _, _| {
                window.show_preferences();
            });
            klass.install_action("win.check-vault", None, |window, _, _| {
                window.check_vault();
            });
            klass.install_action(
                "win.show-block",
                Some(glib::VariantTy::new("(ss)").expect("(ss) is a variant type")),
                |window, _, block| {
                    let (date, id): (String, String) = block
                        .and_then(glib::Variant::get)
                        .expect("blocks are passed as date and id");
                    let date = date.parse().expect("dates are passed as YYYY-MM-DD");
                    let id: BlockId = id.parse().expect("block ids are passed as they are");
                    window.show_block(date, &id);
                },
            );
            klass.install_action(
                "win.show-note",
                Some(glib::VariantTy::STRING),
                |window, _, note| window.show_note(&param(note, "notes")),
            );
            klass.install_action(
                "win.show-task",
                Some(glib::VariantTy::STRING),
                |window, _, id| {
                    let id: TaskId = param(id, "tasks");
                    window.show_tasks();
                    window.imp().tasks_page.show_task(&id);
                },
            );
            klass.install_action(
                "win.show-day",
                Some(glib::VariantTy::STRING),
                |window, _, date| window.show_day(param(date, "days")),
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            style_switcher::add_to(&self.welcome_menu_button);
            style_switcher::add_to(&self.vault_menu_button);
            self.split_view.set_content(Some(&self.day_view));
            self.projects_page.connect_vault_changed(glib::clone!(
                #[weak(rename_to = window)]
                self.obj(),
                move |page| window.projects_changed(page.vault())
            ));
            self.projects_page
                .connect_shown_project_changed(glib::clone!(
                    #[weak(rename_to = window)]
                    self.obj(),
                    move |_| window.select_project_row()
                ));
            // The projects follow the pages, apart.
            self.sidebar_list.set_header_func(glib::clone!(
                #[weak(rename_to = projects_row)]
                self.projects_row,
                move |row, before| {
                    let first_project = before.is_some_and(|before| *before == projects_row);
                    row.set_header(
                        first_project
                            .then(|| {
                                gtk::Separator::builder()
                                    .margin_top(6)
                                    .margin_bottom(6)
                                    .build()
                            })
                            .as_ref(),
                    );
                }
            ));
            // Renaming a note may change links in the day shown.
            self.projects_page.connect_days_changed(glib::clone!(
                #[weak(rename_to = window)]
                self.obj(),
                move |_| window.imp().day_view.reload()
            ));
            self.notes_page.connect_days_changed(glib::clone!(
                #[weak(rename_to = window)]
                self.obj(),
                move |_| window.imp().day_view.reload()
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

    /// Lists the active projects of `vault` in the sidebar in their order,
    /// apart from breaks. The others are on the project page.
    fn show_sidebar_projects(&self, vault: &Vault) {
        let imp = self.imp();
        for (_, row) in imp.project_rows.take() {
            imp.sidebar_list.remove(&row);
        }
        let rows = vault
            .projects()
            .iter()
            .filter(|project| project.status == ProjectStatus::Active && !project.is_break())
            .map(|project| {
                let row = sidebar_project_row(&project.name, &project.color);
                row.set_action_name(Some("win.show-project"));
                row.set_action_target_value(Some(&project.slug.to_string().to_variant()));
                imp.sidebar_list.append(&row);
                (project.slug.clone(), row)
            })
            .collect();
        imp.project_rows.replace(rows);
        self.select_project_row();
    }

    /// Selects the row of the project shown in the sidebar, or that of the
    /// project page if the project has none, while the project page is
    /// shown.
    fn select_project_row(&self) {
        let imp = self.imp();
        if !self.shows(&imp.projects_page) {
            return;
        }
        let shown = imp.projects_page.shown_project();
        let rows = imp.project_rows.borrow();
        let row = rows
            .iter()
            .find(|(slug, _)| Some(slug) == shown.as_ref())
            .map_or(&*imp.projects_row, |(_, row)| row);
        imp.sidebar_list.select_row(Some(row));
    }

    /// Hands `vault`, with projects changed on the project page, to the
    /// other pages. It shares the record of own writes with the vault
    /// watched, so the watcher stays.
    fn projects_changed(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        self.set_vault(&vault);
        imp.day_view.show_date(imp.day_view.date());
    }

    /// Whether `page` is the page shown.
    fn shows(&self, page: &impl IsA<adw::NavigationPage>) -> bool {
        self.imp().split_view.content().as_ref() == Some(page.upcast_ref())
    }

    /// Shows `page` with `row` selected in the sidebar.
    fn show_page(&self, page: &impl IsA<adw::NavigationPage>, row: &gtk::ListBoxRow) {
        let imp = self.imp();
        imp.split_view.set_content(Some(page));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(row));
    }

    /// Shows the day, week or month `steps` steps after the one shown now.
    /// Like going to today, it does nothing on the task and project pages.
    fn step(&self, steps: i32) {
        let imp = self.imp();
        if self.shows(&imp.calendar_view) {
            imp.calendar_view.step(steps);
        } else if self.shows(&imp.reports_page) {
            imp.reports_page.step(steps);
        } else if self.shows(&imp.day_view) {
            let date = imp.day_view.date();
            let date = date.checked_add_signed(TimeDelta::days(steps.into()));
            imp.day_view
                .show_date(date.expect("nobody steps this way to the end of the calendar"));
        }
    }

    /// Shows the day, week or month of today, on the page shown.
    fn step_to_today(&self) {
        let today = Local::now().date_naive();
        let imp = self.imp();
        if self.shows(&imp.calendar_view) {
            imp.calendar_view.show(today);
        } else if self.shows(&imp.reports_page) {
            imp.reports_page.show(today);
        } else if self.shows(&imp.day_view) {
            imp.day_view.show_date(today);
        }
    }

    /// Adds a block to the day shown, forwarded from the window so that the
    /// shortcut works wherever the focus is.
    fn new_block(&self) {
        let day_view = &self.imp().day_view;
        if self.shows(day_view) {
            // Fails on days without a file, which have no blocks yet.
            let _ = WidgetExt::activate_action(day_view, "day.new-block", None);
        }
    }

    fn show_day(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.projects_page.save_now();
        imp.notes_page.save_now();
        imp.day_view.show_date(date);
        self.show_page(&imp.day_view, &imp.today_row);
    }

    fn show_block(&self, date: NaiveDate, id: &BlockId) {
        self.show_day(date);
        self.imp().day_view.show_block_by_id(id);
    }

    fn show_calendar(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.calendar_view.reload();
        self.show_page(&imp.calendar_view, &imp.calendar_row);
    }

    fn show_tasks(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.tasks_page.reload();
        self.show_page(&imp.tasks_page, &imp.tasks_row);
    }

    fn show_notes(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.notes_page.reload();
        self.show_page(&imp.notes_page, &imp.notes_row);
    }

    fn show_projects(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.projects_page.reload();
        self.show_page(&imp.projects_page, &imp.projects_row);
    }

    fn show_project(&self, slug: &ProjectSlug) {
        // Without reloading the page first, which would show the project
        // open before once more.
        self.save_texts_now();
        let imp = self.imp();
        self.show_page(&imp.projects_page, &imp.projects_row);
        imp.projects_page.open_project(slug);
        self.select_project_row();
    }

    /// Shows `note` on the notes page if that is shown, which shows notes
    /// itself, or else in its project.
    fn show_note(&self, note: &NotePath) {
        let imp = self.imp();
        if self.shows(&imp.notes_page) {
            imp.notes_page.open_note(note);
            return;
        }
        self.show_projects();
        imp.projects_page.show_note(note);
        self.select_project_row();
    }

    fn show_preferences(&self) {
        if self.visible_dialog().is_some() {
            return;
        }
        let dialog = PreferencesDialog::new(&self.vault());
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog| window.save_preferences(dialog)
        ));
        dialog.present(Some(self));
    }

    fn check_vault(&self) {
        if self.visible_dialog().is_some() {
            return;
        }
        // So that the check sees what was just typed.
        self.save_texts_now();
        let dialog = VaultCheckDialog::new(self.vault());
        dialog.connect_days_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.imp().day_view.reload()
        ));
        dialog.present(Some(self));
    }

    /// Saves the settings `dialog` holds and closes it. On failure it
    /// stays open.
    fn save_preferences(&self, dialog: &PreferencesDialog) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        match vault.update_config(|config| dialog.apply(config)) {
            Ok(_) => {
                dialog.close();
                self.config_changed(Rc::new(vault));
            }
            Err(err) => show_error(
                dialog,
                &gettext("Cannot Save Preferences"),
                &err.to_string(),
            ),
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
        self.reload_shown_pages();
    }

    /// Reads the notes and the reports again if they are shown. Hidden,
    /// they are read when shown.
    fn reload_shown_pages(&self) {
        let imp = self.imp();
        if self.shows(&imp.notes_page) {
            imp.notes_page.reload();
        }
        if self.shows(&imp.reports_page) {
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
        let dialog = SearchDialog::new(
            self.vault(),
            imp.index.borrow().clone(),
            self.shows(&imp.day_view),
        );
        dialog.present(Some(self));
    }

    /// Shows the reports of the period shown before, at first the current
    /// month.
    fn show_reports(&self) {
        let imp = self.imp();
        self.save_texts_now();
        imp.reports_page.reload();
        self.show_page(&imp.reports_page, &imp.reports_row);
    }

    /// Saves what is being typed on any page, if there are unsaved changes.
    fn save_texts_now(&self) {
        let imp = self.imp();
        imp.day_view.save_texts_now();
        imp.projects_page.save_now();
        imp.notes_page.save_now();
    }
}

/// A project in the sidebar, marked by its colour as the pages above are by
/// their icons.
fn sidebar_project_row(name: &str, color: &str) -> gtk::ListBoxRow {
    let content = gtk::Box::builder().spacing(12).build();
    content.append(
        &gtk::Label::builder()
            .label(color_dot(color))
            .use_markup(true)
            // As wide as the icons above.
            .width_request(16)
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(name)
            .tooltip_text(name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    gtk::ListBoxRow::builder().child(&content).build()
}
