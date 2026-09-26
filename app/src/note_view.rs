use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;
use knotbook_core::{NoteFile, NotePath, ReadError, SavedNote, Vault};

use crate::conflict_dialog::ConflictDialog;
use crate::markdown_view::MarkdownView;
use crate::project_view::note_menu;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/note_view.ui")]
    pub struct NoteView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The note shown, as last read or saved.
        pub file: RefCell<Option<NoteFile>>,
        /// Saves what is being typed a moment after the last key.
        pub save: RefCell<Option<glib::SourceId>>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub editor: TemplateChild<MarkdownView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteView {
        const NAME: &'static str = "KnotbookNoteView";
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
        let project = self.vault().project(note.project()).map_or_else(
            || note.project().to_string(),
            |project| project.name.clone(),
        );
        self.set_title(note.name());
        imp.window_title.set_title(note.name());
        imp.window_title.set_subtitle(&project);
        imp.menu_button.set_menu_model(Some(&note_menu(note)));
        let vault = self.vault();
        imp.editor
            .set_wiki_links(note.project().clone(), move |note| {
                vault.note_path(note).is_file()
            });
        // A new note starts with an empty undo history.
        imp.editor.set_markdown(&file.text);
        imp.file.replace(Some(file));
        Ok(())
    }

    /// Shows the note as it is now, after it was changed elsewhere. What is
    /// being typed is saved instead, which shows both versions if they
    /// differ, or creates the note again if it was removed. Returns whether
    /// the note is still there.
    pub fn reload(&self) -> bool {
        let imp = self.imp();
        if imp.save.borrow().is_some() {
            self.save_now();
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
                true
            }
            Err(ReadError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => false,
            Err(err) => {
                glib::g_warning!("knotbook", "{err}");
                true
            }
        }
    }

    /// Marks the wiki links again, after notes were added or removed.
    pub fn update_links(&self) {
        self.imp().editor.restyle();
    }

    /// Stops showing the note without saving what is being typed, as when
    /// it is deleted.
    pub fn forget(&self) {
        let imp = self.imp();
        if let Some(source) = imp.save.take() {
            source.remove();
        }
        imp.file.replace(None);
    }

    fn save_later(&self) {
        let imp = self.imp();
        if let Some(source) = imp.save.take() {
            source.remove();
        }
        let source = glib::timeout_add_local_once(
            Duration::from_secs(1),
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move || {
                    view.imp().save.take();
                    view.save();
                }
            ),
        );
        imp.save.replace(Some(source));
    }

    /// Saves what is being typed, if there are unsaved changes.
    pub fn save_now(&self) {
        if let Some(source) = self.imp().save.take() {
            source.remove();
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
            Err(err) => {
                let alert = adw::AlertDialog::new(
                    Some(&gettext("Cannot Save Note")),
                    Some(&err.to_string()),
                );
                alert.add_response("close", &gettext("_Close"));
                alert.present(Some(self));
            }
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
