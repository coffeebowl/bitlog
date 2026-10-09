use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::Vault;
use bitlog_index::{Found, IndexError, SearchHit};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::format::format_full_date;
use crate::search_index::SearchIndex;
use crate::window::show_action;

/// Search results shown at most.
const LIMIT: u32 = 50;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/search_dialog.ui")]
    pub struct SearchDialog {
        #[template_child]
        pub entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub status: TemplateChild<adw::StatusPage>,
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub index: RefCell<SearchIndex>,
        pub commands: RefCell<Vec<Command>>,
        /// Counts the searches started, so that the results of one that
        /// finishes after a newer one are dropped.
        pub searches: Cell<u32>,
        /// Why the index cannot be used, once it is known.
        pub error: RefCell<Option<String>>,
        /// The action of each row shown, with its target.
        pub actions: RefCell<Vec<RowAction>>,
        /// The action chosen and the window to run it in once the dialog is
        /// closed.
        pub chosen: RefCell<Option<(gtk::Root, RowAction)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchDialog {
        const NAME: &'static str = "BitLogSearchDialog";
        type Type = super::SearchDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SearchDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.entry.connect_search_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.search()
            ));
            self.entry.connect_activate(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    if let Some(row) = dialog.imp().list.selected_row() {
                        dialog.choose(&row);
                    }
                }
            ));
            self.entry.connect_stop_search(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    dialog.close();
                }
            ));
            // The arrow keys choose a row while the focus stays in the
            // entry, to go on typing.
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                dialog,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| match key {
                    gdk::Key::Down => dialog.select_by(1),
                    gdk::Key::Up => dialog.select_by(-1),
                    _ => glib::Propagation::Proceed,
                }
            ));
            self.entry.add_controller(keys);
            self.list.connect_row_activated(glib::clone!(
                #[weak]
                dialog,
                move |_, row| dialog.choose(row)
            ));
            // Only now, as closing gives the focus back to where it was
            // before, which would take it from a page the action shows.
            dialog.connect_closed(|dialog| {
                if let Some((window, (action, target))) = dialog.imp().chosen.take() {
                    WidgetExt::activate_action(&window, action, target.as_ref())
                        .expect("the rows run actions of the window or app");
                }
            });
        }
    }

    impl WidgetImpl for SearchDialog {}
    impl AdwDialogImpl for SearchDialog {}
}

glib::wrapper! {
    /// Finds texts in the vault and runs commands, opened with Ctrl+K.
    pub struct SearchDialog(ObjectSubclass<imp::SearchDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

/// An action name with its target.
type RowAction = (&'static str, Option<glib::Variant>);

/// Something the dialog offers to do without searching.
#[derive(Debug)]
pub struct Command {
    title: String,
    icon: &'static str,
    action: &'static str,
}

/// The commands, with "New Block" only if `new_block`, when a day is shown.
fn commands(new_block: bool) -> Vec<Command> {
    let command = |title: String, icon, action| Command {
        title,
        icon,
        action,
    };
    let mut commands = vec![
        command(gettext("Today"), "weather-clear-symbolic", "win.show-today"),
        command(
            gettext("Calendar"),
            "x-office-calendar-symbolic",
            "win.show-calendar",
        ),
        command(
            gettext("Tasks"),
            "checkbox-checked-symbolic",
            "win.show-tasks",
        ),
        command(gettext("Projects"), "folder-symbolic", "win.show-projects"),
        command(gettext("Reports"), "view-grid-symbolic", "win.show-reports"),
    ];
    if new_block {
        commands.push(command(
            gettext("New Block"),
            "list-add-symbolic",
            "win.new-block",
        ));
    }
    commands.extend([
        command(
            gettext("New Project"),
            "list-add-symbolic",
            "win.new-project",
        ),
        command(
            gettext("Open Vault…"),
            "folder-open-symbolic",
            "win.open-vault",
        ),
        command(
            gettext("New Vault…"),
            "folder-new-symbolic",
            "win.new-vault",
        ),
        command(gettext("About BitLog"), "help-about-symbolic", "app.about"),
    ]);
    commands
}

impl SearchDialog {
    /// A dialog searching `vault` through `index`. It brings the index up to
    /// date first, which takes a moment the first time.
    pub fn new(vault: Rc<Vault>, index: SearchIndex, new_block: bool) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.commands.replace(commands(new_block));
        imp.vault.replace(Some(vault.clone()));
        imp.index.replace(index.clone());
        dialog.show_results("", Ok(Vec::new()));
        glib::spawn_future_local(glib::clone!(
            #[weak]
            dialog,
            async move {
                if let Err(err) = index.update(&vault).await {
                    dialog.imp().error.replace(Some(err.to_string()));
                }
                dialog.search();
            }
        ));
        dialog
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the dialog is set up with a vault")
    }

    /// Shows the commands and texts that match what is typed.
    fn search(&self) {
        let imp = self.imp();
        let query = imp.entry.text().trim().to_owned();
        let search = imp.searches.get() + 1;
        imp.searches.set(search);
        if query.is_empty() || imp.error.borrow().is_some() {
            let error = imp.error.borrow().clone();
            self.show_results(&query, error.map_or(Ok(Vec::new()), Err));
            return;
        }
        let vault = self.vault();
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let hits = index.search(&vault, query.clone(), LIMIT).await;
                if dialog.imp().searches.get() == search {
                    dialog.show_results(&query, hits.map_err(|err: IndexError| err.to_string()));
                }
            }
        ));
    }

    /// Lists the commands matching `query`, then `hits`, and selects the
    /// first row.
    fn show_results(&self, query: &str, hits: Result<Vec<SearchHit>, String>) {
        let imp = self.imp();
        imp.list.remove_all();
        let query = query.to_lowercase();
        let vault = self.vault();
        let mut rows = Vec::new();
        for command in imp.commands.borrow().iter() {
            let title = command.title.to_lowercase();
            if query.split_whitespace().all(|word| title.contains(word)) {
                rows.push(command_row(command));
            }
        }
        let error = match hits {
            Ok(hits) => {
                rows.extend(hits.iter().map(|hit| hit_row(&vault, hit)));
                None
            }
            Err(err) => Some(err),
        };
        let mut actions = Vec::new();
        for (row, action) in rows {
            imp.list.append(&row);
            actions.push(action);
        }
        imp.actions.replace(actions);
        match imp.list.row_at_index(0) {
            Some(row) => {
                imp.list.select_row(Some(&row));
                imp.scrolled.vadjustment().set_value(0.0);
                imp.stack.set_visible_child_name("results");
            }
            None => {
                let (title, description) = match &error {
                    Some(err) => (gettext("Cannot Search"), err.clone()),
                    None => (gettext("No Results"), String::new()),
                };
                imp.status.set_title(&title);
                imp.status.set_description(Some(&description));
                imp.stack.set_visible_child_name("empty");
            }
        }
    }

    /// Closes the dialog to run the action of `row`.
    fn choose(&self, row: &gtk::ListBoxRow) {
        let imp = self.imp();
        let index = usize::try_from(row.index()).expect("rows in the list have an index");
        let action = imp.actions.borrow()[index].clone();
        let window = self.root().expect("an open dialog lies in a window");
        imp.chosen.replace(Some((window, action)));
        self.close();
    }

    /// Selects the row `steps` rows below the selected one, or above for
    /// negative steps, and scrolls to it.
    fn select_by(&self, steps: i32) -> glib::Propagation {
        let imp = self.imp();
        let index = imp.list.selected_row().map_or(-1, |row| row.index());
        let Some(row) = imp.list.row_at_index(index + steps) else {
            return glib::Propagation::Stop;
        };
        imp.list.select_row(Some(&row));
        if let Some(bounds) = row.compute_bounds(&*imp.list) {
            let top = f64::from(bounds.y());
            imp.scrolled
                .vadjustment()
                .clamp_page(top, top + f64::from(bounds.height()));
        }
        glib::Propagation::Stop
    }
}

fn command_row(command: &Command) -> (adw::ActionRow, RowAction) {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&command.title))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(command.icon));
    let accel = gio::Application::default()
        .and_downcast::<gtk::Application>()
        .and_then(|app| app.accels_for_action(command.action).first().cloned());
    if let Some(accel) = accel {
        row.add_suffix(&adw::ShortcutLabel::new(&accel));
    }
    (row, (command.action, None))
}

fn hit_row(vault: &Vault, hit: &SearchHit) -> (adw::ActionRow, RowAction) {
    let (icon, place) = match &hit.found {
        Found::Block { date, .. } => (
            "x-office-calendar-symbolic",
            // Translators: Where a search found something, as in "Block · September 22, 2026".
            gettext("Block · {date}").replace("{date}", &format_full_date(*date)),
        ),
        Found::DayNote(date) => (
            "x-office-calendar-symbolic",
            // Translators: Where a search found something, as in "Day Note · September 22, 2026".
            gettext("Day Note · {date}").replace("{date}", &format_full_date(*date)),
        ),
        Found::Note(note) => (
            "text-x-generic-symbolic",
            // Translators: Where a search found something, as in "Note · Webshop / Ideas".
            gettext("Note · {project} / {name}")
                .replace("{project}", vault.project_name(note.project()))
                .replace("{name}", note.name()),
        ),
        Found::Task(_) => ("checkbox-checked-symbolic", gettext("Task")),
    };
    let (action, target) = show_action(&hit.found);
    let row = adw::ActionRow::builder()
        .title(snippet_markup(hit))
        .title_lines(1)
        .subtitle(glib::markup_escape_text(&place))
        .subtitle_lines(1)
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    (row, (action, Some(target)))
}

/// The snippet of `hit` on one line as Pango markup, the matches in bold.
fn snippet_markup(hit: &SearchHit) -> String {
    let one_line = |text: &str| {
        glib::markup_escape_text(&text.split_whitespace().collect::<Vec<_>>().join(" ")).to_string()
    };
    let mut markup = String::new();
    let mut end = 0;
    for range in &hit.matches {
        markup += &one_line(&hit.snippet[end..range.start]);
        // Keep the spaces around a match that the line breaks leave.
        if hit.snippet[..range.start].ends_with(char::is_whitespace) && !markup.ends_with(' ') {
            markup.push(' ');
        }
        markup += &format!("<b>{}</b>", one_line(&hit.snippet[range.clone()]));
        if hit.snippet[range.end..].starts_with(char::is_whitespace) {
            markup.push(' ');
        }
        end = range.end;
    }
    markup += &one_line(&hit.snippet[end..]);
    markup
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    #[test]
    fn snippet_on_one_line() {
        let hit = |snippet: &str, words: &[&str]| SearchHit {
            found: Found::DayNote(NaiveDate::from_ymd_opt(2026, 9, 23).unwrap()),
            snippet: snippet.to_owned(),
            matches: words
                .iter()
                .map(|word| {
                    let start = snippet.find(word).unwrap();
                    start..start + word.len()
                })
                .collect(),
        };
        assert_eq!(
            snippet_markup(&hit("Release deployment", &["deploy"])),
            "Release <b>deploy</b>ment"
        );
        assert_eq!(
            snippet_markup(&hit(
                "# Deploy\n\nand  <test> \n deploy",
                &["Deploy", "deploy"]
            )),
            "# <b>Deploy</b> and &lt;test&gt; <b>deploy</b>"
        );
        assert_eq!(
            snippet_markup(&hit("tls \n TLS", &["tls", "TLS"])),
            "<b>tls</b> <b>TLS</b>"
        );
    }
}
