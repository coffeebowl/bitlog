//! What the search offers to do, listed before anything is typed.

use bitlog_core::Vault;
use chrono::{Days, Local};
use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;

use crate::widgets::project_picker::choosable_projects;
use crate::widgets::style_switcher;
use crate::window::pages;

/// Something the search offers to do: run `action` with `target`.
#[derive(Debug)]
pub struct Command {
    pub(super) title: String,
    pub(super) icon: &'static str,
    pub(super) action: &'static str,
    pub(super) target: Option<glib::Variant>,
    /// What it would do, like the time a new block takes up.
    pub(super) subtitle: Option<String>,
    /// Offered only once something is typed.
    pub(super) typed: bool,
}

impl Command {
    pub fn new(title: String, icon: &'static str, action: &'static str) -> Self {
        Self {
            title,
            icon,
            action,
            target: None,
            subtitle: None,
            typed: false,
        }
    }

    pub fn target(mut self, target: impl ToVariant) -> Self {
        self.target = Some(target.to_variant());
        self
    }

    pub fn subtitle(mut self, subtitle: String) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// Offers it only once something is typed, for commands there are many
    /// of, like one for each project.
    pub fn when_typed(mut self) -> Self {
        self.typed = true;
        self
    }
}

/// The pages and the days next to today, then `page_commands` for the page
/// shown, then what the main menu offers and new notes in the projects of
/// `vault`.
pub(super) fn commands(page_commands: Vec<Command>, vault: &Vault) -> Vec<Command> {
    let mut commands: Vec<Command> = pages()
        .into_iter()
        .map(|(title, icon, action)| Command::new(title, icon, action))
        .collect();
    let today = Local::now().date_naive();
    for (title, icon, date) in [
        (
            gettext("Yesterday"),
            "go-previous-symbolic",
            today - Days::new(1),
        ),
        (
            gettext("Tomorrow"),
            "go-next-symbolic",
            today + Days::new(1),
        ),
    ] {
        commands.push(Command::new(title, icon, "win.show-day").target(date.to_string()));
    }
    commands.extend(page_commands);
    commands.extend([
        Command::new(
            gettext("New Project"),
            "list-add-symbolic",
            "win.new-project",
        ),
        Command::new(gettext("New Task"), "list-add-symbolic", "win.new-task"),
        Command::new(
            gettext("New Contact"),
            "list-add-symbolic",
            "win.new-contact",
        ),
    ]);
    for project in choosable_projects(vault) {
        commands.push(
            Command::new(
                // Translators: A command of the search, as in "New Note in Webshop".
                gettext("New Note in {project}").replace("{project}", &project.name),
                "list-add-symbolic",
                "win.new-note",
            )
            .target(project.slug.to_string())
            .when_typed(),
        );
    }
    commands.extend([
        Command::new(
            gettext("Open Vault…"),
            "folder-open-symbolic",
            "win.open-vault",
        ),
        Command::new(
            gettext("New Vault…"),
            "folder-new-symbolic",
            "win.new-vault",
        ),
        Command::new(
            gettext("Preferences"),
            "preferences-system-symbolic",
            "win.preferences",
        ),
    ]);
    for (scheme, _, title) in style_switcher::choices() {
        commands.push(
            Command::new(title, "display-brightness-symbolic", "app.color-scheme").target(scheme),
        );
    }
    commands.extend([
        Command::new(
            gettext("Check Vault…"),
            "object-select-symbolic",
            "win.check-vault",
        ),
        Command::new(
            gettext("Keyboard Shortcuts"),
            "input-keyboard-symbolic",
            "app.shortcuts",
        ),
        Command::new(gettext("Help"), "help-browser-symbolic", "app.help"),
        Command::new(gettext("About BitLog"), "help-about-symbolic", "app.about"),
    ]);
    commands
}
