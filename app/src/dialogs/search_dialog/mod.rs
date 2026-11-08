mod commands;
mod fuzzy;
mod usage;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Contact, NotePath, Project, Vault};
use bitlog_index::{Found, IndexError, SearchHit};
use chrono::{NaiveDate, Utc};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::format::format_full_date;
use crate::search_index::SearchIndex;
use crate::window::show_action;
pub use commands::Command;
use usage::Usage;

/// Search results shown at most, and notes listed by name.
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
        /// The page shown, which runs the actions chosen.
        pub page: RefCell<Option<gtk::Widget>>,
        pub commands: RefCell<Vec<Command>>,
        pub usage: RefCell<Usage>,
        /// Counts the searches started, so that the results of one that
        /// finishes after a newer one are dropped.
        pub searches: Cell<u32>,
        /// Why the index cannot be used, once it is known.
        pub error: RefCell<Option<String>>,
        /// The key and the action of each row shown.
        pub shown: RefCell<Vec<(String, RowAction)>>,
        /// The action chosen, to run once the dialog is closed.
        pub chosen: RefCell<Option<RowAction>>,
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
                let imp = dialog.imp();
                if let Some((action, target)) = imp.chosen.take() {
                    let page = imp.page.borrow().clone();
                    let page = page.expect("the dialog is set up with the page shown");
                    WidgetExt::activate_action(&page, action, target.as_ref())
                        .expect("the rows run actions of the page, window or app");
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

/// What a query looks through, chosen by a sign before its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    Commands,
    /// Projects and contacts by their names.
    Names,
    /// Notes by their names.
    Notes,
    /// A new task with the words as its title.
    NewTask,
}

/// The scope of `query` and the words after its sign.
fn scope(query: &str) -> (Scope, &str) {
    let scope = match query.chars().next() {
        Some('>') => Scope::Commands,
        Some('@') => Scope::Names,
        Some('/') => Scope::Notes,
        Some('+') => Scope::NewTask,
        _ => return (Scope::All, query),
    };
    (scope, query[1..].trim_start())
}

impl SearchDialog {
    /// A dialog searching `vault` through `index`, offering `page_commands`
    /// for `page`, the page shown. It brings the index up to date first,
    /// which takes a moment the first time.
    pub fn new(
        vault: Rc<Vault>,
        index: SearchIndex,
        page: &impl IsA<gtk::Widget>,
        page_commands: Vec<Command>,
    ) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.commands
            .replace(commands::commands(page_commands, &vault));
        imp.usage.replace(Usage::load());
        imp.page.replace(Some(page.clone().upcast()));
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

    /// Shows what matches what is typed, with texts from the index unless
    /// a sign limits the search.
    fn search(&self) {
        let imp = self.imp();
        let query = imp.entry.text().trim().to_owned();
        let search = imp.searches.get() + 1;
        imp.searches.set(search);
        if query.is_empty() || scope(&query).0 != Scope::All {
            self.show_results(&query, Ok(Vec::new()));
            return;
        }
        let error = imp.error.borrow().clone();
        if let Some(error) = error {
            self.show_results(&query, Err(error));
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

    /// Lists the commands, projects, contacts and notes matching `query`,
    /// as far as its scope allows, then `hits`, ordered by `rank`, and
    /// selects the first row.
    fn show_results(&self, query: &str, hits: Result<Vec<SearchHit>, String>) {
        let imp = self.imp();
        imp.list.remove_all();
        let (scope, rest) = scope(query);
        let words = rest.to_lowercase();
        let words: Vec<&str> = words.split_whitespace().collect();
        let typed = !words.is_empty();
        let vault = self.vault();
        let usage = imp.usage.borrow();
        let now = Utc::now().timestamp();
        let mut entries = Vec::new();
        if matches!(scope, Scope::All | Scope::Commands) {
            let commands = imp.commands.borrow();
            let offered = commands.iter().filter(|command| typed || !command.typed);
            entries.extend(offered.filter_map(|c| Entry::command(c).matching(&c.title, &words)));
        }
        if scope == Scope::Names || (scope == Scope::All && typed) {
            let projects = vault.projects().iter();
            entries.extend(projects.filter_map(|p| project_entry(p).matching(&p.name, &words)));
            let contacts = vault.contacts().iter();
            entries.extend(contacts.filter_map(|c| contact_entry(c).matching(&c.name, &words)));
        }
        if scope == Scope::Notes {
            let mut notes: Vec<Entry> = vault
                .all_notes()
                .flatten()
                .filter_map(|note| note_entry(&vault, &note).matching(note.name(), &words))
                .collect();
            rank(&mut notes, &usage, now);
            notes.truncate(LIMIT as usize);
            entries.extend(notes);
        }
        if scope == Scope::NewTask && typed {
            entries.push(task_entry(rest));
        }
        if scope == Scope::All
            && let Ok(date) = query.parse()
        {
            entries.push(date_entry(date));
        }
        let error = match hits {
            Ok(hits) => {
                entries.extend(hits.iter().map(|hit| hit_entry(&vault, hit)));
                None
            }
            Err(err) => Some(err),
        };
        rank(&mut entries, &usage, now);
        let mut shown = Vec::new();
        for entry in entries {
            imp.list.append(&entry.row());
            shown.push((entry.key, entry.action));
        }
        imp.shown.replace(shown);
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

    /// Closes the dialog to run the action of `row`, which counts as chosen
    /// once more.
    fn choose(&self, row: &gtk::ListBoxRow) {
        let imp = self.imp();
        let index = usize::try_from(row.index()).expect("rows in the list have an index");
        let (key, action) = imp.shown.borrow()[index].clone();
        {
            let mut usage = imp.usage.borrow_mut();
            usage.record(&key, Utc::now().timestamp());
            usage.save();
        }
        imp.chosen.replace(Some(action));
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

/// A row to show, with the action it runs.
struct Entry {
    /// What the usage knows it by: a command by its title, as some targets
    /// change, like the date of yesterday; anything else by its action and
    /// target.
    key: String,
    /// In Pango markup.
    title: String,
    /// What was found and where, or what a command would do.
    place: Option<String>,
    icon: &'static str,
    action: RowAction,
    /// How well its name matches what is typed, 0 for texts found.
    quality: u32,
}

impl Entry {
    fn command(command: &Command) -> Self {
        Self {
            key: command.title.clone(),
            title: glib::markup_escape_text(&command.title).into(),
            place: command.subtitle.clone(),
            icon: command.icon,
            action: (command.action, command.target.clone()),
            quality: 0,
        }
    }

    fn found(
        title: String,
        place: String,
        icon: &'static str,
        (action, target): (&'static str, glib::Variant),
    ) -> Self {
        Self {
            key: format!("{action} {}", target.print(false)),
            title,
            place: Some(place),
            icon,
            action: (action, Some(target)),
            quality: 0,
        }
    }

    /// The entry with how well `name` matches `words`, if it does.
    fn matching(mut self, name: &str, words: &[&str]) -> Option<Self> {
        self.quality = fuzzy::score(name, words)?;
        Some(self)
    }

    fn row(&self) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&self.title)
            .title_lines(1)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(self.icon));
        if let Some(place) = &self.place {
            row.set_subtitle(&glib::markup_escape_text(place));
            row.set_subtitle_lines(1);
        }
        let accel = gio::Application::default()
            .and_downcast::<gtk::Application>()
            .and_then(|app| app.accels_for_action(self.action.0).first().cloned());
        if let Some(accel) = accel {
            row.add_suffix(&adw::ShortcutLabel::new(&accel));
        }
        row
    }
}

/// Puts the best matches first, and of those equally good what was chosen
/// more often and lately, the rest in its order.
fn rank(entries: &mut [Entry], usage: &Usage, now: i64) {
    entries.sort_by(|a, b| {
        let usage = |entry: &Entry| usage.score(&entry.key, now);
        b.quality
            .cmp(&a.quality)
            .then_with(|| usage(b).total_cmp(&usage(a)))
    });
}

fn project_entry(project: &Project) -> Entry {
    Entry::found(
        glib::markup_escape_text(&project.name).into(),
        gettext("Project"),
        "project-symbolic",
        ("win.show-project", project.slug.to_string().to_variant()),
    )
}

fn contact_entry(contact: &Contact) -> Entry {
    Entry::found(
        glib::markup_escape_text(&contact.name).into(),
        gettext("Contact"),
        "avatar-default-symbolic",
        ("win.show-contact", contact.slug.to_string().to_variant()),
    )
}

fn note_entry(vault: &Vault, note: &NotePath) -> Entry {
    Entry::found(
        glib::markup_escape_text(note.name()).into(),
        // Translators: A note found by its name, as in "Note · Webshop".
        gettext("Note · {project}").replace("{project}", vault.project_name(note.project())),
        "note-symbolic",
        show_action(&Found::Note(note.clone())),
    )
}

fn task_entry(title: &str) -> Entry {
    Entry::found(
        // Translators: A command of the search, as in "New Task: Call Kim".
        glib::markup_escape_text(&gettext("New Task: {title}").replace("{title}", title)).into(),
        gettext("Due today"),
        "list-add-symbolic",
        ("win.add-task", title.to_variant()),
    )
}

fn date_entry(date: NaiveDate) -> Entry {
    Entry::found(
        glib::markup_escape_text(&format_full_date(date)).into(),
        gettext("Day"),
        "calendar-symbolic",
        ("win.show-day", date.to_string().to_variant()),
    )
}

fn hit_entry(vault: &Vault, hit: &SearchHit) -> Entry {
    let (icon, place) = match &hit.found {
        Found::Block { date, .. } => (
            "calendar-symbolic",
            // Translators: Where a search found something, as in "Block · September 22, 2026".
            gettext("Block · {date}").replace("{date}", &format_full_date(*date)),
        ),
        Found::DayNote(date) => (
            "calendar-symbolic",
            // Translators: Where a search found something, as in "Day Note · September 22, 2026".
            gettext("Day Note · {date}").replace("{date}", &format_full_date(*date)),
        ),
        Found::Note(note) => (
            "note-symbolic",
            // Translators: Where a search found something, as in "Note · Webshop / Ideas".
            gettext("Note · {project} / {name}")
                .replace("{project}", vault.project_name(note.project()))
                .replace("{name}", note.name()),
        ),
        Found::Task { project: None, .. } => ("circle-check-symbolic", gettext("Task")),
        Found::Task {
            project: Some(project),
            ..
        } => (
            "circle-check-symbolic",
            // Translators: Where a search found something, as in "Task · Webshop".
            gettext("Task · {project}").replace("{project}", vault.project_name(project)),
        ),
    };
    Entry::found(snippet_markup(hit), place, icon, show_action(&hit.found))
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
    fn signs_limit_the_scope() {
        assert_eq!(scope("deploy"), (Scope::All, "deploy"));
        assert_eq!(scope("> dark"), (Scope::Commands, "dark"));
        assert_eq!(scope("@kim"), (Scope::Names, "kim"));
        assert_eq!(scope("/"), (Scope::Notes, ""));
        assert_eq!(scope("+Call Kim"), (Scope::NewTask, "Call Kim"));
    }

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
