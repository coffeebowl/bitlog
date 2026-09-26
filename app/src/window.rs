use std::path::Path;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Local, NaiveDate};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use knotbook_core::{ReadError, Vault};

use crate::config;

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/window.ui")]
    pub struct Window {
        pub settings: gio::Settings,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub vault_page: TemplateChild<adw::StatusPage>,
    }

    impl Default for Window {
        fn default() -> Self {
            Self {
                settings: gio::Settings::new(config::app_id()),
                stack: TemplateChild::default(),
                vault_page: TemplateChild::default(),
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
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {}
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
    /// On failure the window goes back to the vault selection.
    fn open_vault(&self, path: &Path) {
        let imp = self.imp();
        match Vault::open(path).and_then(|vault| describe(&vault).map(|text| (vault, text))) {
            Ok((vault, description)) => {
                imp.vault_page.set_title(&vault.config().name);
                imp.vault_page.set_description(Some(&description));
                imp.stack.set_visible_child_name("vault");
                imp.settings
                    .set_string("last-vault", &gio::File::for_path(path).uri())
                    .expect("the last vault can be stored");
            }
            Err(err) => {
                imp.stack.set_visible_child_name("welcome");
                self.show_error(&err.to_string());
            }
        }
    }

    fn show_error(&self, message: &str) {
        let dialog = adw::AlertDialog::new(Some(&gettext("Cannot Open Vault")), Some(message));
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
    }
}

/// A short summary of `vault`: where it lies and what it holds.
///
/// Returned as Pango markup for the description of a status page.
fn describe(vault: &Vault) -> Result<String, ReadError> {
    let projects = vault.projects().len();
    let year = Local::now().year();
    let days = vault
        .days(
            NaiveDate::from_ymd_opt(year, 1, 1).expect("January 1 exists"),
            NaiveDate::from_ymd_opt(year, 12, 31).expect("December 31 exists"),
        )?
        .len();
    let counts = [
        // Translators: {count} is the number of projects.
        ngettext("{count} project", "{count} projects", count(projects))
            .replace("{count}", &projects.to_string()),
        // Translators: {count} is a number of days, {year} the current year.
        ngettext(
            "{count} day logged in {year}",
            "{count} days logged in {year}",
            count(days),
        )
        .replace("{count}", &days.to_string())
        .replace("{year}", &year.to_string()),
    ];
    Ok(format!(
        "{}\n{}",
        glib::markup_escape_text(&vault.root().display().to_string()),
        glib::markup_escape_text(&counts.join(" · ")),
    ))
}

/// Clamps `n` for gettext's plural selection, which only takes `u32`.
fn count(n: usize) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}
