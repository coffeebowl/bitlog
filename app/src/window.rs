use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, TimeDelta};
use gettextrs::gettext;
use gtk::{gio, glib};
use knotbook_core::Vault;

use crate::config;
use crate::day_view::DayView;

/// Actions that need an open vault.
const VAULT_ACTIONS: [&str; 3] = ["win.previous-day", "win.next-day", "win.today"];

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
        pub day_view: TemplateChild<DayView>,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::app_id()),
                stack: TemplateChild::default(),
                split_view: TemplateChild::default(),
                sidebar: TemplateChild::default(),
                day_view: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "KnotbookWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            DayView::ensure_type();
            klass.bind_template();
            klass.install_action_async("win.open-vault", None, |window, _, _| async move {
                window.choose_vault().await;
            });
            klass.install_action("win.previous-day", None, |window, _, _| {
                window.move_days(-1);
            });
            klass.install_action("win.next-day", None, |window, _, _| {
                window.move_days(1);
            });
            klass.install_action("win.today", None, |window, _, _| {
                let imp = window.imp();
                imp.day_view.show_date(Local::now().date_naive());
                imp.split_view.set_show_content(true);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
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
        imp.day_view.set_vault(vault);
        imp.stack.set_visible_child_name("vault");
        for action in VAULT_ACTIONS {
            self.action_set_enabled(action, true);
        }
        imp.settings
            .set_string("last-vault", &gio::File::for_path(path).uri())
            .expect("the last vault can be stored");
    }

    /// Shows the day `days` days after the one shown now.
    fn move_days(&self, days: i64) {
        let day_view = &self.imp().day_view;
        let date = day_view
            .date()
            .checked_add_signed(TimeDelta::days(days))
            .expect("nobody steps day by day to the end of the calendar");
        day_view.show_date(date);
    }

    fn show_error(&self, message: &str) {
        let dialog = adw::AlertDialog::new(Some(&gettext("Cannot Open Vault")), Some(message));
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
    }
}
