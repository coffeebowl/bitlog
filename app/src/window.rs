use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gio, glib};
use knotbook_core::Vault;

use crate::calendar_view::CalendarView;
use crate::config;
use crate::day_view::DayView;

/// Actions that need an open vault.
const VAULT_ACTIONS: [&str; 6] = [
    "win.previous",
    "win.next",
    "win.today",
    "win.show-today",
    "win.show-calendar",
    "win.show-day",
];

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/window.ui")]
    pub struct Window {
        pub settings: gio::Settings,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub sidebar_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub today_row: TemplateChild<gtk::ListBoxRow>,
        #[template_child]
        pub calendar_row: TemplateChild<gtk::ListBoxRow>,
        /// The pages of the content, owned here because only one of them is
        /// in the split view at a time.
        pub day_view: DayView,
        pub calendar_view: CalendarView,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::app_id()),
                stack: TemplateChild::default(),
                split_view: TemplateChild::default(),
                sidebar: TemplateChild::default(),
                sidebar_list: TemplateChild::default(),
                today_row: TemplateChild::default(),
                calendar_row: TemplateChild::default(),
                day_view: glib::Object::new(),
                calendar_view: glib::Object::new(),
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
            // Previous, next and today move by the unit of the page shown.
            klass.install_action("win.previous", None, |window, _, _| window.step(-1));
            klass.install_action("win.next", None, |window, _, _| window.step(1));
            klass.install_action("win.today", None, |window, _, _| {
                let today = Local::now().date_naive();
                let imp = window.imp();
                if window.shows_calendar() {
                    imp.calendar_view.show(today);
                } else {
                    imp.day_view.show_date(today);
                }
            });
            klass.install_action("win.show-today", None, |window, _, _| {
                window.show_day(Local::now().date_naive());
            });
            klass.install_action("win.show-calendar", None, |window, _, _| {
                window.show_calendar();
            });
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
            None => self.show_error(&gettext("The vault is not on a local file system.")),
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
            None => self.show_error(&gettext("Choose a folder on a local file system.")),
        }
    }

    /// Opens the vault in `path` and remembers it for the next start.
    ///
    /// On failure the window keeps showing what it showed before.
    fn open_vault(&self, path: &Path) {
        let imp = self.imp();
        let vault = match Vault::open(path) {
            Ok(vault) => Rc::new(vault),
            Err(err) => {
                self.show_error(&err.to_string());
                return;
            }
        };
        imp.sidebar.set_title(&vault.config().name);
        imp.calendar_view.set_vault(vault.clone());
        imp.day_view.set_vault(vault);
        imp.split_view.set_content(Some(&imp.day_view));
        imp.sidebar_list.select_row(Some(&*imp.today_row));
        imp.stack.set_visible_child_name("vault");
        for action in VAULT_ACTIONS {
            self.action_set_enabled(action, true);
        }
        imp.settings
            .set_string("last-vault", &gio::File::for_path(path).uri())
            .expect("the last vault can be stored");
    }

    fn shows_calendar(&self) -> bool {
        let imp = self.imp();
        imp.split_view.content().as_ref() == Some(imp.calendar_view.upcast_ref())
    }

    /// Shows the day, week or month `steps` steps after the one shown now.
    fn step(&self, steps: i32) {
        let imp = self.imp();
        if self.shows_calendar() {
            imp.calendar_view.step(steps);
        } else {
            let date = imp.day_view.date();
            let date = date.checked_add_signed(TimeDelta::days(steps.into()));
            imp.day_view
                .show_date(date.expect("nobody steps this way to the end of the calendar"));
        }
    }

    fn show_day(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.day_view.show_date(date);
        imp.split_view.set_content(Some(&imp.day_view));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.today_row));
    }

    fn show_calendar(&self) {
        let imp = self.imp();
        imp.calendar_view.reload();
        imp.split_view.set_content(Some(&imp.calendar_view));
        imp.split_view.set_show_content(true);
        imp.sidebar_list.select_row(Some(&*imp.calendar_row));
    }

    fn show_error(&self, message: &str) {
        let dialog = adw::AlertDialog::new(Some(&gettext("Cannot Open Vault")), Some(message));
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
    }
}
