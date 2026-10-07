//! Opening, reading and watching the vault, and its sync conflicts.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{NotePath, ProjectSlug, ReadError, Vault, VaultChange, WatchError};
use chrono::Local;
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};

use super::{VAULT_ACTIONS, Window};
use crate::alert::show_error;
use crate::format::plural;
use crate::search_index::SearchIndex;
use crate::sync_conflict_dialog::{SyncConflictDialog, file_title};
use crate::vault_check_dialog::count_severe;

impl Window {
    /// Opens the vault of the last session, if there is one.
    pub(super) fn open_last_vault(&self) {
        let uri = self.imp().settings.string("last-vault");
        if uri.is_empty() {
            return;
        }
        let folder = gio::File::for_uri(&uri);
        match folder.path() {
            Some(path) => self.open_vault(&path),
            None => show_error(
                self,
                &gettext("Cannot Open Vault"),
                &gettext("The vault is not on a local file system."),
            ),
        }
    }

    pub(super) async fn choose_vault(&self) {
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
            None => show_error(
                self,
                &gettext("Cannot Open Vault"),
                &gettext("Choose a folder on a local file system."),
            ),
        }
    }

    /// Creates a vault in a folder the user chooses and opens it.
    pub(super) async fn create_vault(&self) {
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
            show_error(
                self,
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
            .map_or("BitLog".into(), |name| name.to_string_lossy());
        match Vault::create(path, &name, Local::now().date_naive()) {
            Ok(_) => self.open_vault(path),
            Err(err) => show_error(self, &gettext("Cannot Create Vault"), &err.to_string()),
        }
    }

    /// Opens the vault in `path` and remembers it for the next start.
    ///
    /// On failure the window keeps showing what it showed before.
    fn open_vault(&self, path: &Path) {
        let imp = self.imp();
        if let Err(err) = self.load_vault(path) {
            show_error(self, &gettext("Cannot Open Vault"), &err.to_string());
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
        self.report_severe_problems();
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
        imp.notes_page.set_index(index.clone());
        imp.reports_page.set_index(index.clone());
        let indexed = vault.clone();
        glib::spawn_future_local(async move {
            if let Err(err) = index.update(&indexed).await {
                glib::g_warning!("bitlog", "{err}");
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
                let message = format!(
                    "{}\n\n{err}",
                    gettext("Changes made elsewhere only show up after going to another day.")
                );
                show_error(self, &gettext("Cannot Watch Vault"), &message);
            }
        }
        Ok(())
    }

    pub(super) fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("only an open vault can be worked on")
    }

    pub(super) fn set_vault(&self, vault: &Rc<Vault>) {
        let imp = self.imp();
        imp.vault.replace(Some(vault.clone()));
        imp.sidebar.set_title(&vault.config().name);
        imp.calendar_view.set_vault(vault.clone());
        imp.day_view.set_vault(vault.clone());
        imp.tasks_page.set_vault(vault.clone());
        imp.notes_page.set_vault(vault.clone());
        imp.projects_page.set_vault(vault.clone());
        imp.reports_page.set_vault(vault.clone());
        self.show_sidebar_projects(vault);
    }

    /// Shows what was changed elsewhere, by sync or the CLI.
    fn vault_changed(&self, changes: Result<Vec<VaultChange>, WatchError>) {
        match changes {
            Ok(changes) => {
                self.show_changes(&changes);
                self.check_conflicts();
            }
            // Watching goes on, a later change may be seen again.
            Err(err) => glib::g_warning!("bitlog", "{err}"),
        }
    }

    /// Merges the sync conflict copies without contradictions, and offers
    /// to resolve the others.
    fn check_conflicts(&self) {
        let imp = self.imp();
        let vault = self.vault();
        let copies = match vault.conflict_copies() {
            Ok(copies) => copies,
            Err(err) => {
                glib::g_warning!("bitlog", "{err}");
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
                // An unreadable copy cannot be resolved here; `bitlog
                // doctor` names it.
                Err(err) => glib::g_warning!("bitlog", "{err}"),
            }
        }
        imp.conflict_banner.set_title(
            &ngettext(
                "{count} sync conflict",
                "{count} sync conflicts",
                plural(open.len()),
            )
            .replace("{count}", &open.len().to_string()),
        );
        if let Some(first) = open.first() {
            let banner = &imp.conflict_banner;
            // The target first, as the action takes one.
            banner.set_action_target_value(Some(&first.path.to_string_lossy().to_variant()));
            banner.set_action_name(Some("win.resolve-conflict"));
        }
        imp.conflict_banner.set_revealed(!open.is_empty());
    }

    /// Lets the user resolve the sync conflict of the copy at `path`,
    /// relative to the vault.
    pub(super) fn resolve_conflict(&self, path: &Path) {
        let copy = match self.vault().conflict_copies() {
            Ok(copies) => copies.into_iter().find(|copy| copy.path == path),
            Err(err) => {
                show_error(
                    self,
                    &gettext("Cannot Read Sync Conflict"),
                    &err.to_string(),
                );
                return;
            }
        };
        // Resolved meanwhile, by sync or on another device.
        let Some(copy) = copy else {
            self.check_conflicts();
            return;
        };
        self.save_texts_now();
        match SyncConflictDialog::new(self.vault(), copy.clone()) {
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
            Err(err) => show_error(
                self,
                &gettext("Cannot Read Sync Conflict"),
                &err.to_string(),
            ),
        }
    }

    /// Checks the vault in the background and reports severe problems,
    /// which may go unnoticed for long otherwise.
    fn report_severe_problems(&self) {
        let vault = self.vault();
        let path = self.imp().vault_path.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let severe = count_severe(&vault).await;
                // Another vault may be open by now.
                if *window.imp().vault_path.borrow() != path {
                    return;
                }
                match severe {
                    Ok(0) => {}
                    Ok(count) => window.show_severe_problems(count),
                    Err(err) => glib::g_warning!("bitlog", "{err}"),
                }
            }
        ));
    }

    /// Tells about `count` severe problems in the vault, to be looked at in
    /// the vault check.
    fn show_severe_problems(&self, count: usize) {
        let toast = adw::Toast::builder()
            .title(
                ngettext(
                    "{count} severe problem found",
                    "{count} severe problems found",
                    plural(count),
                )
                .replace("{count}", &count.to_string()),
            )
            .button_label(gettext("_Show"))
            .action_name("win.check-vault")
            .build();
        self.imp().toast_overlay.add_toast(toast);
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
            if self.shows(&imp.tasks_page) {
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
            imp.day_view.update_links();
            imp.projects_page.notes_changed(&notes);
            if self.shows(&imp.notes_page) {
                imp.notes_page.reload();
            }
        }
        let assets: Vec<ProjectSlug> = changes
            .iter()
            .filter_map(|change| match change {
                VaultChange::Assets(slug) => Some(slug.clone()),
                _ => None,
            })
            .collect();
        if !assets.is_empty() {
            imp.projects_page.assets_changed(&assets);
        }
        let changes_day = |change: &VaultChange| matches!(change, VaultChange::Day(_));
        if self.shows(&imp.calendar_view) && changes.iter().any(changes_day) {
            imp.calendar_view.reload();
        }
        if self.shows(&imp.reports_page) && changes.iter().any(changes_day) {
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
                self.reload_shown_pages();
                self.check_conflicts();
            }
            Err(err) => show_error(self, &gettext("Cannot Open Vault"), &err.to_string()),
        }
    }
}
