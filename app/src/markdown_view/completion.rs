//! Suggesting notes while a wiki link is typed, and contacts while a mention
//! is, in the completion popover of the source view, fuzzy matched as in
//! GNOME Builder.

use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{link_target, typed_link_target, typed_mention};
use gtk::{gio, glib};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;
use sourceview5::{Completion, CompletionCell, CompletionColumn, CompletionContext};

use super::editing::replace_range;
use super::{MarkdownView, char_offsets, lists};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Proposal {
        /// What the list shows and what is typed is matched against.
        pub shown: RefCell<String>,
        /// What choosing the proposal puts in the text.
        pub inserted: RefCell<String>,
        /// Shown after it, if not empty.
        pub after: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Proposal {
        const NAME: &'static str = "BitLogProposal";
        type Type = super::Proposal;
        type Interfaces = (sourceview5::CompletionProposal,);
    }

    impl ObjectImpl for Proposal {}
    impl CompletionProposalImpl for Proposal {}

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
            let store = gio::ListStore::new::<super::Proposal>();
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
            display(&self.query.borrow(), proposal, cell);
        }

        fn activate(
            &self,
            _context: &CompletionContext,
            proposal: &sourceview5::CompletionProposal,
        ) {
            if let Some(view) = self.view.upgrade() {
                view.insert_link_target(&inserted(proposal));
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
                    let score = fuzzy_match(&[&target], &needle)?;
                    let is_other = wiki.project.as_ref() != Some(note.project());
                    Some((is_other, score, glib::casefold(&target), target))
                })
                .collect();
            matches.sort();
            let proposals: Vec<super::Proposal> = matches
                .into_iter()
                .map(|(_, _, _, target)| super::Proposal::new(&target, &target, ""))
                .collect();
            store.splice(0, store.n_items(), &proposals);
            self.query.replace(query);
        }
    }

    #[derive(Debug, Default)]
    pub struct ContactProvider {
        pub view: glib::WeakRef<MarkdownView>,
        /// What was typed of the mention when the proposals were last
        /// filtered, to highlight it in them.
        pub query: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactProvider {
        const NAME: &'static str = "BitLogContactProvider";
        type Type = super::ContactProvider;
        type Interfaces = (sourceview5::CompletionProvider,);
    }

    impl ObjectImpl for ContactProvider {}

    impl CompletionProviderImpl for ContactProvider {
        /// `@` and dashes are no part of words, see `NoteProvider`.
        fn is_trigger(&self, _iter: &gtk::TextIter, _c: char) -> bool {
            self.view
                .upgrade()
                .is_some_and(|view| view.typed_mention().is_some())
        }

        fn populate(&self, _context: &CompletionContext) -> Result<gio::ListModel, glib::Error> {
            let store = gio::ListStore::new::<super::Proposal>();
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
            display(&self.query.borrow(), proposal, cell);
        }

        fn activate(
            &self,
            _context: &CompletionContext,
            proposal: &sourceview5::CompletionProposal,
        ) {
            if let Some(view) = self.view.upgrade() {
                view.insert_mention(&inserted(proposal));
            }
        }
    }

    impl ContactProvider {
        /// Fills `store` with the contacts whose name or slug matches what
        /// is typed of the mention, those of the project of the text first,
        /// the best matches first. Archived contacts are left out. Empty if
        /// no mention is being typed.
        fn fill(&self, store: &gio::ListStore) {
            let view = self.view.upgrade();
            let query = view.as_ref().and_then(MarkdownView::typed_mention);
            let (Some(view), Some(query)) = (view, query) else {
                store.remove_all();
                return;
            };
            let needle = glib::casefold(&query);
            let contacts = view.imp().contacts.borrow();
            let contacts = contacts
                .as_ref()
                .expect("mentions are only typed where contacts are set");
            let mut matches: Vec<_> = contacts
                .contacts
                .iter()
                .filter(|contact| !contact.archived)
                .filter_map(|contact| {
                    let score = fuzzy_match(&[&contact.name, contact.slug.as_str()], &needle)?;
                    let is_other = contacts
                        .project
                        .as_ref()
                        .is_none_or(|project| contact.role(project).is_none());
                    Some((is_other, score, glib::casefold(&contact.name), contact))
                })
                .collect();
            matches.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
            let proposals: Vec<super::Proposal> = matches
                .into_iter()
                .map(|(_, _, _, contact)| {
                    let slug = contact.slug.as_str();
                    super::Proposal::new(&contact.name, slug, &format!("@{slug}"))
                })
                .collect();
            store.splice(0, store.n_items(), &proposals);
            self.query.replace(query);
        }
    }

    /// The best score of `texts` for `needle`, casefolded, where lower is
    /// better, or `None` if none matches. Anything matches nothing typed.
    fn fuzzy_match(texts: &[&str], needle: &str) -> Option<u32> {
        if needle.is_empty() {
            return Some(0);
        }
        texts
            .iter()
            .filter_map(|text| Completion::fuzzy_match(Some(text), needle))
            .min()
    }

    /// Shows `proposal` in `cell`, with what matches `query` highlighted.
    fn display(query: &str, proposal: &sourceview5::CompletionProposal, cell: &CompletionCell) {
        let proposal = proposal_of(proposal);
        match cell.column() {
            CompletionColumn::TypedText => {
                let shown = proposal.shown.borrow();
                let query = glib::casefold(query);
                match Completion::fuzzy_highlight(&shown, &query) {
                    Some(attributes) => cell.set_text_with_attributes(&shown, &attributes),
                    None => cell.set_text(Some(&shown)),
                }
            }
            CompletionColumn::After => cell.set_text(Some(&proposal.after.borrow())),
            _ => {}
        }
    }

    fn proposal_of(proposal: &sourceview5::CompletionProposal) -> &Proposal {
        proposal
            .downcast_ref::<super::Proposal>()
            .expect("the providers only propose their own")
            .imp()
    }

    fn inserted(proposal: &sourceview5::CompletionProposal) -> String {
        proposal_of(proposal).inserted.borrow().clone()
    }
}

glib::wrapper! {
    /// A note to link to or a contact to mention.
    pub struct Proposal(ObjectSubclass<imp::Proposal>)
        @implements sourceview5::CompletionProposal;
}

impl Proposal {
    fn new(shown: &str, inserted: &str, after: &str) -> Self {
        let proposal: Self = glib::Object::new();
        let imp = proposal.imp();
        imp.shown.replace(shown.to_owned());
        imp.inserted.replace(inserted.to_owned());
        imp.after.replace(after.to_owned());
        proposal
    }
}

glib::wrapper! {
    /// Proposes the notes of the vault while a wiki link is typed.
    pub struct NoteProvider(ObjectSubclass<imp::NoteProvider>)
        @implements sourceview5::CompletionProvider;
}

glib::wrapper! {
    /// Proposes the contacts of the vault while a mention is typed.
    pub struct ContactProvider(ObjectSubclass<imp::ContactProvider>)
        @implements sourceview5::CompletionProvider;
}

impl MarkdownView {
    /// Proposes notes while a wiki link is typed, once the view has its
    /// wiki links, and contacts while a mention is, once it has contacts.
    pub(super) fn suggest_notes_and_contacts(&self) {
        let notes: NoteProvider = glib::Object::new();
        notes.imp().view.set(Some(self));
        let contacts: ContactProvider = glib::Object::new();
        contacts.imp().view.set(Some(self));
        let completion = self.completion();
        completion.set_select_on_show(true);
        completion.set_show_icons(false);
        completion.add_provider(&notes);
        completion.add_provider(&contacts);
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

    /// What is typed of the mention at the cursor after its `@`, if the
    /// cursor is in one and the view has contacts.
    fn typed_mention(&self) -> Option<String> {
        if !self.is_editable() || self.imp().contacts.borrow().is_none() {
            return None;
        }
        let (text, at) = lists::text_and_cursor(&self.buffer());
        let range = typed_mention(&text, at)?;
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
        let mut start = replace_range(&buffer, offsets[range.start]..offsets[range.end], target);
        if closing < 2 && !rest.starts_with(['|', '#']) {
            buffer.insert(&mut start, &"]".repeat(2 - closing));
        }
        start.forward_chars(closing as i32);
        buffer.place_cursor(&start);
        buffer.end_user_action();
    }

    /// Makes `slug` the contact of the mention at the cursor and puts the
    /// cursor after it.
    fn insert_mention(&self, slug: &str) {
        let buffer = self.buffer();
        let (text, at) = lists::text_and_cursor(&buffer);
        let Some(range) = typed_mention(&text, at) else {
            return;
        };
        let offsets = char_offsets(&text);
        buffer.begin_user_action();
        let end = replace_range(&buffer, offsets[range.start]..offsets[range.end], slug);
        buffer.place_cursor(&end);
        buffer.end_user_action();
    }
}
