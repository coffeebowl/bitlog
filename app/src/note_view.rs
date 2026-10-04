use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{NoteFile, NotePath, ReadError, SavedNote, Vault};
use bitlog_index::{Backlink, Found};
use gettextrs::{gettext, ngettext};
use gtk::glib;

use crate::alert::show_error;
use crate::conflict_dialog::ConflictDialog;
use crate::format::{format_full_date, plural, title_markup};
use crate::markdown_view::MarkdownView;
use crate::project_view::note_menu;
use crate::search_index::SearchIndex;
use crate::widgets::SaveTimer;
use crate::window::show_action;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/note_view.ui")]
    pub struct NoteView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub index: RefCell<SearchIndex>,
        /// The places linking to the note, as the popover lists them.
        pub backlinks: RefCell<Vec<Backlink>>,
        /// Counts the lookups of backlinks, so that an older one finishing
        /// late is dropped.
        pub lookups: Cell<u32>,
        /// The note shown, as last read or saved.
        pub file: RefCell<Option<NoteFile>>,
        /// Saves what is being typed a moment after the last key.
        pub save: SaveTimer,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub editor: TemplateChild<MarkdownView>,
        #[template_child]
        pub backlinks_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub backlinks_list: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteView {
        const NAME: &'static str = "BitLogNoteView";
        type Type = super::NoteView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            MarkdownView::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for NoteView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            self.editor.connect_edited(glib::clone!(
                #[weak]
                view,
                move |_| view.save_later()
            ));
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(glib::clone!(
                #[weak]
                view,
                move |_| view.save_now()
            ));
            self.editor.add_controller(focus);
            self.editor.connect_wiki_link_activated(glib::clone!(
                #[weak]
                view,
                move |note| {
                    let target = note.to_string().to_variant();
                    // Handled by the project page, which may create the note.
                    let _ = WidgetExt::activate_action(&view, "notes.follow", Some(&target));
                }
            ));
            view.connect_hiding(|view| view.save_now());
            self.backlinks_list.connect_row_activated(glib::clone!(
                #[weak]
                view,
                move |_, row| view.follow_backlink(row.index())
            ));
        }
    }

    impl WidgetImpl for NoteView {}
    impl NavigationPageImpl for NoteView {}
}

glib::wrapper! {
    /// A project note in the editor, saved while it is typed.
    pub struct NoteView(ObjectSubclass<imp::NoteView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl NoteView {
    /// Uses `vault` from now on, after saving what is being typed.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.save_now();
        self.imp().vault.replace(Some(vault));
    }

    /// Uses `index` to find the notes and days linking to the note shown.
    pub fn set_index(&self, index: SearchIndex) {
        self.imp().index.replace(index);
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("notes are only shown with a vault")
    }

    /// The note shown, if any.
    pub fn note(&self) -> Option<NotePath> {
        self.imp()
            .file
            .borrow()
            .as_ref()
            .map(|file| file.path.clone())
    }

    /// Reads the note `note` and shows it, after saving the one shown.
    pub fn show_note(&self, note: &NotePath) -> Result<(), ReadError> {
        self.save_now();
        let file = self.vault().load_note(note)?;
        let imp = self.imp();
        self.set_title(note.name());
        imp.window_title.set_title(note.name());
        imp.window_title
            .set_subtitle(self.vault().project_name(note.project()));
        imp.menu_button.set_menu_model(Some(&note_menu(note)));
        imp.editor
            .set_wiki_links(note.project().clone(), existing_notes(&self.vault()));
        // A new note starts with an empty undo history.
        imp.editor.set_markdown(&file.text);
        imp.file.replace(Some(file));
        // Those of the note shown before are wrong until the new ones are in.
        imp.backlinks_button.set_visible(false);
        self.show_backlinks();
        Ok(())
    }

    /// Shows the note as it is now, after it was changed elsewhere. What is
    /// being typed is saved instead, which shows both versions if they
    /// differ, or creates the note again if it was removed. Returns whether
    /// the note is still there.
    pub fn reload(&self) -> bool {
        let imp = self.imp();
        if imp.save.cancel() {
            self.save();
            return true;
        }
        let Some(note) = self.note() else {
            return false;
        };
        match self.vault().load_note(&note) {
            Ok(file) => {
                // Keeps the cursor where nothing changed.
                if !imp.editor.shows(&file.text) {
                    imp.editor.set_markdown(&file.text);
                }
                imp.file.replace(Some(file));
                self.show_backlinks();
                true
            }
            Err(ReadError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => false,
            Err(err) => {
                glib::g_warning!("bitlog", "{err}");
                true
            }
        }
    }

    /// Marks the wiki links again and looks up the links to the note, after
    /// notes were changed, added or removed.
    pub fn update_links(&self) {
        if let Some(note) = self.note() {
            self.imp()
                .editor
                .set_wiki_links(note.project().clone(), existing_notes(&self.vault()));
            self.show_backlinks();
        }
    }

    /// Looks up the places linking to the note shown in the background and
    /// lists them in the popover, whose button shows how many there are.
    fn show_backlinks(&self) {
        let imp = self.imp();
        let note = self
            .note()
            .expect("backlinks are looked up for a note shown");
        let lookup = imp.lookups.get() + 1;
        imp.lookups.set(lookup);
        let index = imp.index.borrow().clone();
        let vault = self.vault();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let backlinks = index.backlinks(&vault, note).await;
                let imp = view.imp();
                if imp.lookups.get() != lookup {
                    return;
                }
                let backlinks = backlinks.unwrap_or_else(|err| {
                    glib::g_warning!("bitlog", "{err}");
                    Vec::new()
                });
                imp.backlinks_list.remove_all();
                for backlink in &backlinks {
                    imp.backlinks_list.append(&backlink_row(&vault, backlink));
                }
                let label = ngettext("{count} Link", "{count} Links", plural(backlinks.len()))
                    .replace("{count}", &backlinks.len().to_string());
                imp.backlinks_button.set_label(&label);
                imp.backlinks_button.set_visible(!backlinks.is_empty());
                imp.backlinks.replace(backlinks);
            }
        ));
    }

    /// Shows the place of the backlink in row `index` of the popover.
    fn follow_backlink(&self, index: i32) {
        let imp = self.imp();
        let index = usize::try_from(index).expect("rows in the list have an index");
        let (action, target) = show_action(&imp.backlinks.borrow()[index].found);
        imp.backlinks_button.popdown();
        WidgetExt::activate_action(self, action, Some(&target))
            .expect("the window shows notes and days");
    }

    /// Stops showing the note without saving what is being typed, as when
    /// it is deleted.
    pub fn forget(&self) {
        let imp = self.imp();
        imp.save.cancel();
        imp.file.replace(None);
    }

    fn save_later(&self) {
        self.imp().save.schedule(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.save()
        ));
    }

    /// Saves what is being typed, if there are unsaved changes.
    pub fn save_now(&self) {
        if self.imp().save.cancel() {
            self.save();
        }
    }

    /// Saves the text. On failure the typed text stays, so that nothing
    /// gets lost.
    fn save(&self) {
        let imp = self.imp();
        let Some(file) = imp.file.borrow().clone() else {
            return;
        };
        let text = imp.editor.markdown();
        match self.vault().save_note(&file, &text) {
            Ok(SavedNote::Saved(saved)) => {
                imp.file.replace(Some(saved));
            }
            Ok(SavedNote::Conflict(theirs)) => self.choose_version(text, theirs),
            Err(err) => show_error(self, &gettext("Cannot Save Note"), &err.to_string()),
        }
    }

    /// Lets the user choose between `mine`, the text typed, and `theirs`,
    /// the note as changed elsewhere.
    fn choose_version(&self, mine: String, theirs: NoteFile) {
        let dialog = ConflictDialog::new(theirs.path.name(), &mine, &theirs.text);
        dialog.connect_chosen(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |keep_mine| {
                let imp = view.imp();
                let kept = if keep_mine { &mine } else { &theirs.text };
                if !imp.editor.shows(kept) {
                    imp.editor.set_markdown(kept);
                }
                imp.file.replace(Some(theirs.clone()));
                if keep_mine {
                    // Saves over the other version, or asks again if it
                    // changed once more.
                    view.save();
                }
            }
        ));
        dialog.present(Some(self));
    }
}

/// The notes of all projects of `vault`, to tell which wiki links point to
/// one. Projects whose notes cannot be listed count as having none.
fn existing_notes(vault: &Vault) -> HashSet<NotePath> {
    vault
        .all_notes()
        .filter_map(|note| {
            note.inspect_err(|err| glib::g_warning!("bitlog", "{err}"))
                .ok()
        })
        .collect()
}

/// A row naming the note, block or day note of `backlink`.
fn backlink_row(vault: &Vault, backlink: &Backlink) -> adw::ActionRow {
    let escaped = |text: &str| glib::markup_escape_text(text).to_string();
    let (title, subtitle) = match &backlink.found {
        Found::Note(note) => (
            escaped(note.name()),
            vault.project_name(note.project()).to_owned(),
        ),
        Found::Block { date, .. } => {
            let project = backlink.project.as_ref().map_or_else(
                || gettext("Block"),
                |project| vault.project_name(project).to_owned(),
            );
            let title = backlink.title.as_deref().unwrap_or_default();
            (title_markup(title, &project), format_full_date(*date))
        }
        Found::DayNote(date) => (escaped(&gettext("Day Note")), format_full_date(*date)),
        Found::Task(_) => unreachable!("tasks hold no links"),
    };
    adw::ActionRow::builder()
        .title(title)
        .subtitle(glib::markup_escape_text(&subtitle))
        .activatable(true)
        .build()
}
