//! Suggesting notes while a wiki link is typed, in the completion popover of
//! the source view, fuzzy matched as in GNOME Builder.

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{link_target, typed_link_target};
use gtk::{gio, glib};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;
use sourceview5::{Completion, CompletionCell, CompletionColumn, CompletionContext};

use super::{MarkdownView, char_offsets, lists};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct NoteProposal {
        pub target: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteProposal {
        const NAME: &'static str = "BitLogNoteProposal";
        type Type = super::NoteProposal;
        type Interfaces = (sourceview5::CompletionProposal,);
    }

    impl ObjectImpl for NoteProposal {}
    impl CompletionProposalImpl for NoteProposal {}

    #[derive(Debug, Default)]
    pub struct NoteProvider {
        pub view: glib::WeakRef<MarkdownView>,
        /// What was typed of the target when the proposals were last
        /// filtered, to highlight it in them.
        pub query: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteProvider {
        const NAME: &'static str = "BitLogNoteProvider";
        type Type = super::NoteProvider;
        type Interfaces = (sourceview5::CompletionProvider,);
    }

    impl ObjectImpl for NoteProvider {}

    impl CompletionProviderImpl for NoteProvider {
        /// Spaces, slashes and dashes end words, and with them the
        /// completion, unless a provider wants it to go on.
        fn is_trigger(&self, _iter: &gtk::TextIter, _c: char) -> bool {
            self.view
                .upgrade()
                .is_some_and(|view| view.typed_link().is_some())
        }

        fn populate(&self, _context: &CompletionContext) -> Result<gio::ListModel, glib::Error> {
            let store = gio::ListStore::new::<super::NoteProposal>();
            self.fill(&store);
            Ok(store.upcast())
        }

        fn refilter(&self, _context: &CompletionContext, model: &gio::ListModel) {
            if let Some(store) = model.downcast_ref::<gio::ListStore>() {
                self.fill(store);
            }
        }

        fn display(
            &self,
            _context: &CompletionContext,
            proposal: &sourceview5::CompletionProposal,
            cell: &CompletionCell,
        ) {
            if cell.column() != CompletionColumn::TypedText {
                return;
            }
            let target = target_of(proposal);
            let query = glib::casefold(&*self.query.borrow());
            match Completion::fuzzy_highlight(&target, &query) {
                Some(attributes) => cell.set_text_with_attributes(&target, &attributes),
                None => cell.set_text(Some(&target)),
            }
        }

        fn activate(
            &self,
            _context: &CompletionContext,
            proposal: &sourceview5::CompletionProposal,
        ) {
            if let Some(view) = self.view.upgrade() {
                view.insert_link_target(&target_of(proposal));
            }
        }
    }

    impl NoteProvider {
        /// Fills `store` with the notes matching what is typed of the
        /// target, those of the project of the text first, the best matches
        /// first. Empty if no wiki link is being typed.
        fn fill(&self, store: &gio::ListStore) {
            let view = self.view.upgrade();
            let query = view.as_ref().and_then(MarkdownView::typed_link);
            let (Some(view), Some(query)) = (view, query) else {
                store.remove_all();
                return;
            };
            let needle = glib::casefold(&query);
            let wiki_links = view.imp().wiki_links.borrow();
            let wiki = wiki_links
                .as_ref()
                .expect("wiki links are only typed where they are set");
            let mut matches: Vec<(bool, u32, glib::GString, String)> = wiki
                .existing
                .iter()
                .filter_map(|note| {
                    let target = link_target(note, wiki.project.as_ref());
                    let score = if needle.is_empty() {
                        0
                    } else {
                        Completion::fuzzy_match(Some(&target), &needle)?
                    };
                    let is_other = wiki.project.as_ref() != Some(note.project());
                    Some((is_other, score, glib::casefold(&target), target))
                })
                .collect();
            matches.sort();
            let proposals: Vec<super::NoteProposal> = matches
                .into_iter()
                .map(|(_, _, _, target)| super::NoteProposal::new(target))
                .collect();
            store.splice(0, store.n_items(), &proposals);
            self.query.replace(query);
        }
    }

    fn target_of(proposal: &sourceview5::CompletionProposal) -> String {
        proposal
            .downcast_ref::<super::NoteProposal>()
            .expect("the provider only proposes notes")
            .imp()
            .target
            .borrow()
            .clone()
    }
}

glib::wrapper! {
    /// A note to link to, as the link names it.
    pub struct NoteProposal(ObjectSubclass<imp::NoteProposal>)
        @implements sourceview5::CompletionProposal;
}

impl NoteProposal {
    fn new(target: String) -> Self {
        let proposal: Self = glib::Object::new();
        proposal.imp().target.replace(target);
        proposal
    }
}

glib::wrapper! {
    /// Proposes the notes of the vault while a wiki link is typed.
    pub struct NoteProvider(ObjectSubclass<imp::NoteProvider>)
        @implements sourceview5::CompletionProvider;
}

impl MarkdownView {
    /// Proposes notes while a wiki link is typed, once the view has its
    /// wiki links.
    pub(super) fn suggest_notes(&self) {
        let provider: NoteProvider = glib::Object::new();
        provider.imp().view.set(Some(self));
        let completion = self.completion();
        completion.set_select_on_show(true);
        completion.set_show_icons(false);
        completion.add_provider(&provider);
    }

    /// Shows the notes to link to, if a wiki link was just started.
    pub(super) fn suggest_notes_now(&self) {
        if self.typed_link().is_some() {
            self.completion().show();
        }
    }

    /// What is typed of the target of the wiki link at the cursor, if the
    /// cursor is in one and its links can be followed.
    fn typed_link(&self) -> Option<String> {
        if !self.is_editable() || self.imp().wiki_links.borrow().is_none() {
            return None;
        }
        let (text, at) = lists::text_and_cursor(&self.buffer());
        let range = typed_link_target(&text, at)?;
        Some(text[range.start..at].to_owned())
    }

    /// Makes `target` the target of the wiki link at the cursor, closes the
    /// link if needed and puts the cursor after it, or before its `|text`
    /// or `#heading`.
    fn insert_link_target(&self, target: &str) {
        let buffer = self.buffer();
        let (text, at) = lists::text_and_cursor(&buffer);
        let Some(range) = typed_link_target(&text, at) else {
            return;
        };
        let offsets = char_offsets(&text);
        let rest = &text[range.end..];
        let closing = rest.chars().take(2).take_while(|&c| c == ']').count();
        buffer.begin_user_action();
        let mut start = buffer.iter_at_offset(offsets[range.start]);
        let mut end = buffer.iter_at_offset(offsets[range.end]);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, target);
        if closing < 2 && !rest.starts_with(['|', '#']) {
            buffer.insert(&mut start, &"]".repeat(2 - closing));
        }
        start.forward_chars(closing as i32);
        buffer.place_cursor(&start);
        buffer.end_user_action();
    }
}
