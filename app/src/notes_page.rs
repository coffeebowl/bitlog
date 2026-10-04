use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{NotePath, Project, ProjectSlug, Vault};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::colors::color_dot;
use crate::config;
use crate::format::{
    PROJECT_STATUSES, format_full_date, format_short_date, format_time, format_weekday_date,
};
use crate::markdown_view::MarkdownView;
use crate::note_dialogs::{self, confirm_create, confirm_delete};
use crate::note_view::NoteView;
use crate::project_view::{note_card, note_preview, note_text, show_preview};
use crate::projects_page::shows;
use crate::search_index::SearchIndex;

/// A note in the list.
#[derive(Debug)]
pub struct Listed {
    note: NotePath,
    /// When it was last edited, unless that cannot be read.
    modified: Option<DateTime<Local>>,
}

/// The card of a note in the grid, kept while the note is there, so that
/// its preview is only formatted again when the note changed.
#[derive(Debug)]
pub struct Card {
    child: gtk::FlowBoxChild,
    preview: MarkdownView,
    /// The project and when the note was last edited.
    details: gtk::Label,
}

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/dev/bitlog/BitLog/notes_page.ui")]
    #[properties(wrapper_type = super::NotesPage)]
    pub struct NotesPage {
        /// How the list is ordered: "recent", "name" or "project".
        #[property(get, set)]
        pub sort: RefCell<String>,
        pub settings: gio::Settings,
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// Where the links to a note lie, before it is renamed.
        pub index: RefCell<SearchIndex>,
        /// The notes of all projects, as last read.
        pub notes: RefCell<Vec<Listed>>,
        /// The cards of these notes.
        pub cards: RefCell<HashMap<NotePath, Card>>,
        /// The projects in the dropdown, in its order, `None` for all.
        pub project_items: RefCell<Vec<Option<ProjectSlug>>>,
        /// Set while the dropdown is filled, so that choosing shows nothing.
        pub filling_projects: Cell<bool>,
        /// Pushed on top of the list, owned here because it is only in the
        /// navigation view while shown.
        pub note_view: NoteView,
        #[template_child]
        pub nav: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub overview: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub project_dropdown: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub notes_box: TemplateChild<gtk::Box>,
    }

    impl Default for NotesPage {
        fn default() -> Self {
            Self {
                sort: RefCell::default(),
                settings: gio::Settings::new(config::app_id()),
                vault: RefCell::default(),
                index: RefCell::default(),
                notes: RefCell::default(),
                cards: RefCell::default(),
                project_items: RefCell::default(),
                filling_projects: Cell::default(),
                note_view: glib::Object::new(),
                nav: TemplateChild::default(),
                overview: TemplateChild::default(),
                search_entry: TemplateChild::default(),
                project_dropdown: TemplateChild::default(),
                stack: TemplateChild::default(),
                notes_box: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotesPage {
        const NAME: &'static str = "BitLogNotesPage";
        type Type = super::NotesPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_property_action("notes.sort", "sort");
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

    #[glib::derived_properties]
    impl ObjectImpl for NotesPage {
        fn constructed(&self) {
            self.parent_constructed();
            let page = self.obj();
            self.settings.bind("notes-sort", &*page, "sort").build();
            page.connect_sort_notify(|page| page.show_list());
            self.search_entry.connect_search_changed(glib::clone!(
                #[weak]
                page,
                move |_| page.show_list()
            ));
            // Typing on the list searches.
            self.search_entry
                .set_key_capture_widget(Some(&*self.overview));
            self.project_dropdown
                .set_factory(Some(&label_factory(true)));
            self.project_dropdown
                .set_list_factory(Some(&label_factory(false)));
            self.project_dropdown.connect_selected_notify(glib::clone!(
                #[weak]
                page,
                move |_| {
                    if !page.imp().filling_projects.get() {
                        page.show_list();
                    }
                }
            ));
            // Back on the list, it shows the note as just edited.
            self.overview.connect_showing(glib::clone!(
                #[weak]
                page,
                move |_| {
                    page.imp().note_view.save_now();
                    page.show_notes();
                }
            ));
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted after links in day files were changed, which
                    // watching the vault leaves out as own writes.
                    Signal::builder("days-changed").build(),
                ]
            })
        }
    }

    impl WidgetImpl for NotesPage {}
    impl NavigationPageImpl for NotesPage {}
}

glib::wrapper! {
    /// The notes of all projects, to find, sort and open one.
    pub struct NotesPage(ObjectSubclass<imp::NotesPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn note_param(param: Option<&glib::Variant>) -> NotePath {
    param
        .and_then(|param| param.str()?.parse().ok())
        .expect("note actions take a note path")
}

impl NotesPage {
    /// Shows the notes of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        imp.note_view.set_vault(vault.clone());
        imp.vault.replace(Some(vault));
    }

    /// Uses `index` to find the links to notes.
    pub fn set_index(&self, index: SearchIndex) {
        let imp = self.imp();
        imp.note_view.set_index(index.clone());
        imp.index.replace(index);
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the page is only shown with a vault")
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

    /// Shows the notes as they are now, and the note open, if it is still
    /// there.
    pub fn reload(&self) {
        let imp = self.imp();
        self.show_notes();
        if shows(&imp.nav, "note") {
            imp.note_view.update_links();
            if !imp.note_view.reload() {
                self.close_note();
            }
        }
    }

    /// Goes back to the list of notes.
    pub fn show_overview(&self) {
        let imp = self.imp();
        imp.note_view.save_now();
        imp.nav.pop_to_tag("notes");
    }

    /// Shows the note `note` above the list.
    pub fn open_note(&self, note: &NotePath) {
        let imp = self.imp();
        match imp.note_view.show_note(note) {
            Ok(()) if !shows(&imp.nav, "note") => imp.nav.push(&imp.note_view),
            Ok(()) => {}
            Err(err) => show_error(self, &gettext("Cannot Open Note"), &err.to_string()),
        }
    }

    /// Goes back from the note, which is no longer there.
    fn close_note(&self) {
        let imp = self.imp();
        imp.note_view.forget();
        imp.nav.pop_to_tag("notes");
    }

    /// Reads the notes of all projects and lists them. Projects whose
    /// notes cannot be read are left out.
    fn show_notes(&self) {
        let vault = self.vault();
        let notes: Vec<Listed> = vault
            .projects()
            .iter()
            .flat_map(|project| {
                vault.notes(&project.slug).unwrap_or_else(|err| {
                    glib::g_warning!("bitlog", "{err}");
                    Vec::new()
                })
            })
            .map(|note| {
                let modified = vault.note_modified(&note).map_or_else(
                    |err| {
                        glib::g_warning!("bitlog", "{err}");
                        None
                    },
                    |modified| Some(DateTime::from(modified)),
                );
                Listed { note, modified }
            })
            .collect();
        self.update_cards(&vault, &notes);
        self.imp().notes.replace(notes);
        self.fill_projects();
        self.show_list();
    }

    /// Makes a card for each note of `notes` that has none, shows the
    /// previews as the notes are now, and drops the cards of notes that
    /// are gone.
    fn update_cards(&self, vault: &Vault, notes: &[Listed]) {
        let mut cards = self.imp().cards.borrow_mut();
        let mut kept = HashMap::new();
        for listed in notes {
            let card = cards.remove(&listed.note).unwrap_or_else(|| {
                let preview = note_preview();
                let details = gtk::Label::builder().use_markup(true).build();
                let child = note_card(&listed.note, &preview, Some(&details));
                Card {
                    child,
                    preview,
                    details,
                }
            });
            show_preview(&card.preview, &note_text(vault, &listed.note));
            kept.insert(listed.note.clone(), card);
        }
        *cards = kept;
    }

    /// Offers the projects with notes in the dropdown, keeping the one
    /// chosen.
    fn fill_projects(&self) {
        let imp = self.imp();
        let vault = self.vault();
        let chosen = self.chosen_project();
        let items: Vec<Option<ProjectSlug>> = std::iter::once(None)
            .chain(
                projects_in_order(&vault)
                    .into_iter()
                    .filter(|project| {
                        Some(&project.slug) == chosen.as_ref()
                            || imp
                                .notes
                                .borrow()
                                .iter()
                                .any(|listed| *listed.note.project() == project.slug)
                    })
                    .map(|project| Some(project.slug.clone())),
            )
            .collect();
        let selected = items.iter().position(|item| *item == chosen).unwrap_or(0);
        imp.filling_projects.set(true);
        if *imp.project_items.borrow() != items {
            let labels: Vec<String> = items
                .iter()
                .map(|item| match item {
                    Some(slug) => vault.project_name(slug).to_owned(),
                    None => gettext("All Projects"),
                })
                .collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            imp.project_dropdown
                .set_model(Some(&gtk::StringList::new(&labels)));
            imp.project_items.replace(items);
        }
        imp.project_dropdown
            .set_selected(u32::try_from(selected).expect("projects fit in a list"));
        imp.filling_projects.set(false);
    }

    /// The project chosen in the dropdown, `None` for all.
    fn chosen_project(&self) -> Option<ProjectSlug> {
        let imp = self.imp();
        let index = usize::try_from(imp.project_dropdown.selected()).ok()?;
        imp.project_items.borrow().get(index).cloned().flatten()
    }

    /// Lists the notes read that match the search and the project chosen,
    /// in the order chosen. By project, each project has a group of its
    /// own.
    fn show_list(&self) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        // The cards stay, only their grids go.
        for card in imp.cards.borrow().values() {
            if let Some(grid) = card.child.parent().and_downcast::<gtk::FlowBox>() {
                grid.remove(&card.child);
            }
        }
        while let Some(child) = imp.notes_box.first_child() {
            imp.notes_box.remove(&child);
        }
        let notes = imp.notes.borrow();
        let chosen = self.chosen_project();
        let search = imp.search_entry.text().trim().to_lowercase();
        let mut shown: Vec<&Listed> = notes
            .iter()
            .filter(|listed| chosen.is_none() || Some(listed.note.project()) == chosen.as_ref())
            .filter(|listed| listed.note.name().to_lowercase().contains(&search))
            .collect();
        let order = projects_in_order(&vault);
        let position = |slug: &ProjectSlug| {
            order
                .iter()
                .position(|project| project.slug == *slug)
                .unwrap_or(usize::MAX)
        };
        let name = |listed: &Listed| listed.note.name().to_lowercase();
        let by_project = imp.sort.borrow().as_str() == "project";
        match imp.sort.borrow().as_str() {
            "name" => {
                shown.sort_by_cached_key(|listed| (name(listed), position(listed.note.project())))
            }
            "project" => {
                shown.sort_by_cached_key(|listed| (position(listed.note.project()), name(listed)))
            }
            // Newest first, those without a time last.
            _ => shown
                .sort_by_cached_key(|listed| (std::cmp::Reverse(listed.modified), name(listed))),
        }

        let today = Local::now().date_naive();
        let cards = imp.cards.borrow();
        let mut group: Option<(&ProjectSlug, gtk::FlowBox)> = None;
        for listed in &shown {
            let project = listed.note.project();
            let same_group = match &group {
                Some((slug, _)) => !by_project || *slug == project,
                None => false,
            };
            if !same_group {
                let new_group = adw::PreferencesGroup::new();
                if by_project {
                    new_group.set_title(&glib::markup_escape_text(vault.project_name(project)));
                }
                let grid = self.notes_grid();
                new_group.add(&grid);
                imp.notes_box.append(&new_group);
                group = Some((project, grid));
            }
            let (_, grid) = group.as_ref().expect("a group was added");
            let card = cards.get(&listed.note).expect("every note read has a card");
            show_details(
                &card.details,
                listed,
                vault.project(project),
                !by_project,
                today,
            );
            grid.append(&card.child);
        }
        let page = if notes.is_empty() {
            "empty"
        } else if shown.is_empty() {
            "no-results"
        } else {
            "list"
        };
        imp.stack.set_visible_child_name(page);
    }

    /// A grid of note cards as the project view has, opening the note of
    /// the card activated.
    fn notes_grid(&self) -> gtk::FlowBox {
        let grid = gtk::FlowBox::builder()
            .valign(gtk::Align::Start)
            .homogeneous(true)
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(8)
            .column_spacing(12)
            .row_spacing(18)
            .build();
        grid.connect_child_activated(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_, child| {
                let note = page
                    .imp()
                    .cards
                    .borrow()
                    .iter()
                    .find(|(_, card)| card.child == *child)
                    .map(|(note, _)| note.clone());
                if let Some(note) = note {
                    page.open_note(&note);
                }
            }
        ));
        grid
    }

    /// Opens the note a wiki link points to, or offers to create it.
    async fn follow_link(&self, note: NotePath) {
        let vault = self.vault();
        if vault.note_path(&note).is_file() {
            self.open_note(&note);
            return;
        }
        if !confirm_create(self, &vault, &note).await {
            return;
        }
        match vault.create_note(note.project(), note.name(), Local::now().date_naive()) {
            Ok(note) => self.open_note(&note),
            Err(err) => show_error(self, &gettext("Cannot Create Note"), &err.to_string()),
        }
    }

    async fn rename_note(&self, note: NotePath) {
        let imp = self.imp();
        // Renaming may change the links in the note shown, and the index
        // has to find what was just typed.
        imp.note_view.save_now();
        let index = imp.index.borrow().clone();
        let Some((renamed, updated_links)) =
            note_dialogs::rename_note(self, &self.vault(), &index, &note).await
        else {
            return;
        };
        if updated_links {
            self.emit_by_name::<()>("days-changed", &[]);
        }
        if imp.note_view.note() == Some(note) {
            imp.note_view.forget();
            self.open_note(&renamed);
        } else if shows(&imp.nav, "note") {
            imp.note_view.reload();
        }
        self.show_notes();
    }

    async fn delete_note(&self, note: NotePath) {
        let imp = self.imp();
        if !confirm_delete(self, &note).await {
            return;
        }
        if imp.note_view.note() == Some(note.clone()) {
            self.close_note();
        }
        if let Err(err) = self.vault().delete_note(&note) {
            show_error(self, &gettext("Cannot Delete Note"), &err.to_string());
        }
        self.show_notes();
    }
}

/// The projects of `vault` as the project page lists them: by status, and
/// in their order within each.
fn projects_in_order(vault: &Vault) -> Vec<&Project> {
    PROJECT_STATUSES
        .iter()
        .flat_map(|status| {
            vault
                .projects()
                .iter()
                .filter(move |project| project.status == *status)
        })
        .collect()
}

/// Shows in `details` of the card of `listed` the colour of `project` with
/// its name if `with_project`, and when the note was last edited, as seen
/// `today`. The tooltip tells the time as well.
fn show_details(
    details: &gtk::Label,
    listed: &Listed,
    project: Option<&Project>,
    with_project: bool,
    today: NaiveDate,
) {
    let name = project.map_or_else(|| listed.note.project().to_string(), |p| p.name.clone());
    let mut long = vec![name.clone()];
    // Without the name, the date follows the dot directly.
    let mut short = Vec::new();
    if with_project {
        short.push(glib::markup_escape_text(&name).to_string());
    }
    if let Some(modified) = listed.modified {
        short.push(glib::markup_escape_text(&edited_day(modified.date_naive(), today)).into());
        long.push(edited_text(modified, today));
    }
    let short = short.join(" · ");
    let short = match project {
        Some(project) => format!("{} {short}", color_dot(&project.color)),
        None => short,
    };
    details.set_label(&short);
    details.set_tooltip_text(Some(&long.join(" · ")));
}

/// The day a note was last edited, short enough for a card, as seen
/// `today`.
fn edited_day(date: NaiveDate, today: NaiveDate) -> String {
    if date == today {
        gettext("Today")
    } else if today.pred_opt() == Some(date) {
        gettext("Yesterday")
    } else if date.year() == today.year() {
        format_short_date(date)
    } else {
        format_full_date(date)
    }
}

/// When a note was last edited, as in "edited today at 14:05" or "edited
/// on Tue, Sep 22", as seen `today`.
fn edited_text(modified: DateTime<Local>, today: NaiveDate) -> String {
    let date = modified.date_naive();
    let time = format_time(modified.time());
    if date == today {
        // Translators: When a note was last edited, as in
        // "edited today at 14:05".
        gettext("edited today at {time}").replace("{time}", &time)
    } else if today.pred_opt() == Some(date) {
        // Translators: When a note was last edited, as in
        // "edited yesterday at 14:05".
        gettext("edited yesterday at {time}").replace("{time}", &time)
    } else {
        // Translators: When a note was last edited, as in
        // "edited on Tue, Sep 22".
        gettext("edited on {date}").replace("{date}", &format_weekday_date(date, today))
    }
}

/// Shows the strings of a list, cut short with `ellipsize`, as the button
/// of the project dropdown needs to fit narrow windows.
fn label_factory(ellipsize: bool) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("the dropdown makes list items");
        let label = gtk::Label::builder().xalign(0.0).build();
        if ellipsize {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(12);
        }
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("the dropdown makes list items");
        let label = item.child().and_downcast::<gtk::Label>();
        let text = item.item().and_downcast::<gtk::StringObject>();
        if let (Some(label), Some(text)) = (label, text) {
            label.set_label(&text.string());
        }
    });
    factory
}
