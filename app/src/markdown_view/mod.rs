mod callouts;
mod check_boxes;
mod code_blocks;
mod code_highlight;
mod completion;
mod decorations;
mod diagrams;
mod editing;
mod links;
mod lists;
mod styling;
mod tables;
mod tags;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{
    MarkdownMode, NotePath, ProjectSlug, escape_headings, markdown_formatting, trim_blank_lines,
    wiki_links,
};
use glib::SignalHandlerId;
use glib::subclass::Signal;
use gtk::{gdk, gio, glib};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

use self::code_highlight::CodeHighlighter;
pub use self::code_highlight::{Highlighting, Themes, themes};
use self::decorations::Decorations;
use self::diagrams::Diagrams;
use self::links::Target;
use self::styling::Styling;
use crate::colors::with_alpha;

/// How much Markdown syntax is dimmed, as the alpha of the text colour.
const MARKUP_ALPHA: f32 = 0.45;
/// The background of code and table headers, as the alpha of the text
/// colour.
const CODE_ALPHA: f32 = 0.07;
/// The lines of tables and rules, as the alpha of the text colour.
const GRID_ALPHA: f32 = 0.2;
/// Of code blocks and tables.
const CORNER_RADIUS: f32 = 6.0;
/// The actions that change the text, only enabled while it is editable.
const EDIT_ACTIONS: [&str; 3] = ["markdown.bold", "markdown.italic", "markdown.code"];

/// Tells the wiki links of a text apart, see `MarkdownView::set_wiki_links`.
#[derive(Debug)]
pub struct WikiLinks {
    /// That of the text, which short links like `[[name]]` point into.
    project: Option<ProjectSlug>,
    /// The notes there are, of all projects.
    existing: HashSet<NotePath>,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MarkdownView)]
    pub struct MarkdownView {
        /// Whether headings are formatted. Block texts and day notes keep
        /// them as text, project notes format them.
        #[property(get, set)]
        pub full: Cell<bool>,
        /// Shown dimmed while the text is empty.
        #[property(get, set = Self::set_placeholder)]
        pub placeholder: RefCell<String>,
        /// Whether the text is being replaced, which is no edit.
        pub loading: Cell<bool>,
        /// Set for texts whose wiki links can be followed.
        pub wiki_links: RefCell<Option<WikiLinks>>,
        /// The links in the text as last styled, as character ranges with
        /// where they lead.
        pub(super) links: RefCell<Vec<(Range<i32>, Target)>>,
        /// Connected to the style manager, which outlives the view.
        pub style_handlers: RefCell<Vec<SignalHandlerId>>,
        /// What is drawn beside the text as last styled.
        pub(super) decorations: RefCell<Decorations>,
        /// The starts of what shows its Markdown, as the cursor is at it.
        pub revealed: RefCell<Vec<i32>>,
        /// Whether `MarkdownView::queue_reveal` is waiting to run.
        pub reveal_queued: Cell<bool>,
        /// Whether the text is to be formatted again when idle, as it
        /// changed or the view changed its width.
        pub restyle_queued: Cell<bool>,
        pub(super) diagrams: Diagrams,
        pub highlighter: CodeHighlighter,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MarkdownView {
        const NAME: &'static str = "BitLogMarkdownView";
        type Type = super::MarkdownView;
        type ParentType = sourceview5::View;

        fn class_init(klass: &mut Self::Class) {
            // Code as on GitHub; GTK uses no Ctrl+E in text views.
            let shortcuts = [
                (EDIT_ACTIONS[0], "**", gdk::Key::b),
                (EDIT_ACTIONS[1], "*", gdk::Key::i),
                (EDIT_ACTIONS[2], "`", gdk::Key::e),
            ];
            for (action, marker, key) in shortcuts {
                klass.install_action(action, None, move |view, _, _| view.toggle_marker(marker));
                klass.add_binding_action(key, gdk::ModifierType::CONTROL_MASK, action);
            }
            // Enter alone starts a new line.
            klass.install_action("markdown.follow-link", None, |view, _, _| {
                let buffer = view.buffer();
                let cursor = buffer.iter_at_mark(&buffer.get_insert());
                view.follow_link_at(&cursor);
            });
            klass.add_binding_action(
                gdk::Key::Return,
                gdk::ModifierType::CONTROL_MASK,
                "markdown.follow-link",
            );
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MarkdownView {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted after every change of the text by the user.
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("edited").build(),
                    // Emitted with the path of the note a wiki link points
                    // to, when the user follows it.
                    Signal::builder("wiki-link-activated")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            view.set_editable(false);
            view.set_cursor_visible(false);
            for action in EDIT_ACTIONS {
                view.action_set_enabled(action, false);
            }
            view.connect_editable_notify(|view| {
                view.set_cursor_visible(view.is_editable());
                for action in EDIT_ACTIONS {
                    view.action_set_enabled(action, view.is_editable());
                }
                view.queue_reveal();
            });
            view.connect_has_focus_notify(|view| view.queue_reveal());
            view.buffer().connect_cursor_position_notify(glib::clone!(
                #[weak]
                view,
                move |_| view.queue_reveal()
            ));
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            // Tab indents by two spaces, as in nested lists and most code.
            view.set_insert_spaces_instead_of_tabs(true);
            view.set_tab_width(2);
            view.set_indent_width(2);
            view.add_css_class("markdown-view");
            tags::create(&view.buffer());
            code_highlight::preload();
            view.buffer().connect_changed(glib::clone!(
                #[weak]
                view,
                move |_| {
                    if view.imp().loading.get() {
                        // A note just opened shows formatted right away.
                        view.restyle();
                    } else {
                        view.queue_restyle();
                        view.emit_by_name::<()>("edited", &[]);
                    }
                }
            ));
            view.connect_full_notify(|view| view.restyle());
            view.follow_links_on_click();
            view.toggle_check_boxes_on_click();
            view.edit_by_keys();
            view.close_fences();
            view.suggest_notes();

            let style_manager = adw::StyleManager::default();
            set_style_scheme(&view, &style_manager);
            let on_change = glib::clone!(
                #[weak]
                view,
                move |style_manager: &adw::StyleManager| {
                    set_style_scheme(&view, style_manager);
                    // The theme's new colours are only there once its styles
                    // are applied, after the change itself.
                    glib::idle_add_local_once(glib::clone!(
                        #[weak]
                        view,
                        move || tags::update(view.upcast_ref())
                    ));
                }
            );
            self.style_handlers.replace(vec![
                style_manager.connect_dark_notify(on_change.clone()),
                style_manager.connect_high_contrast_notify(on_change),
            ]);
        }

        fn dispose(&self) {
            let style_manager = adw::StyleManager::default();
            for handler in self.style_handlers.take() {
                style_manager.disconnect(handler);
            }
        }
    }

    impl WidgetImpl for MarkdownView {
        fn map(&self) {
            self.parent_map();
            tags::update(self.obj().upcast_ref());
        }

        /// Fits diagrams and tables to the new width.
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let view = self.obj();
            if self.decorations.borrow().misfit(view.upcast_ref()) {
                view.queue_restyle();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.parent_snapshot(snapshot);
            let view = self.obj();
            let placeholder = self.placeholder.borrow();
            if placeholder.is_empty() || view.buffer().char_count() > 0 {
                return;
            }
            let layout = view.create_pango_layout(Some(&placeholder));
            let start = view.iter_location(&view.buffer().start_iter());
            let (x, y) =
                view.buffer_to_window_coords(gtk::TextWindowType::Widget, start.x(), start.y());
            let color = view.color();
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(x as f32, y as f32));
            snapshot.append_layout(&layout, &with_alpha(&color, MARKUP_ALPHA));
            snapshot.restore();
        }
    }

    impl MarkdownView {
        fn set_placeholder(&self, placeholder: String) {
            self.obj()
                .update_property(&[gtk::accessible::Property::Placeholder(&placeholder)]);
            self.placeholder.replace(placeholder);
            self.obj().queue_draw();
        }
    }

    impl TextViewImpl for MarkdownView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            self.parent_snapshot_layer(layer, snapshot.clone());
            let view = self.obj();
            let decorations = self.decorations.borrow();
            match layer {
                gtk::TextViewLayer::BelowText => {
                    decorations.snapshot_below(view.upcast_ref(), &snapshot);
                }
                gtk::TextViewLayer::AboveText => {
                    decorations.snapshot_above(view.upcast_ref(), &snapshot);
                }
                _ => {}
            }
        }
    }
    impl ViewImpl for MarkdownView {}
}

glib::wrapper! {
    /// Markdown with live formatting: the syntax stays visible, but dimmed,
    /// except for the markers of bullets and quotes, which are drawn as
    /// bullets and bars. Code blocks are cards, highlighted when they name
    /// a language, or diagrams when they are in Mermaid. Tables are grids,
    /// their cells cut off if too wide, and rules are lines, but show their
    /// Markdown while the cursor is in them, tables in a monospace font with
    /// the columns lined up. Enter continues lists, quotes and tables, and
    /// typing a wiki link proposes notes.
    ///
    /// Read-only unless made editable, with a placeholder while empty. When editable, Ctrl+B, Ctrl+I and
    /// Ctrl+E make the selection bold, italic or code, or undo that, and Tab indents by two spaces.
    pub struct MarkdownView(ObjectSubclass<imp::MarkdownView>)
        @extends sourceview5::View, gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Scrollable;
}

impl MarkdownView {
    /// Shows `text`, which cannot be undone.
    pub fn set_markdown(&self, text: &str) {
        let buffer = self.buffer();
        self.imp().loading.set(true);
        buffer.begin_irreversible_action();
        buffer.set_text(text);
        buffer.end_irreversible_action();
        self.imp().loading.set(false);
    }

    pub fn connect_edited(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "edited",
            false,
            glib::closure_local!(move |view: &Self| callback(view)),
        );
    }

    /// Makes the wiki links of this text of `project` followable, and dims
    /// those to notes missing from `existing`. A day note belongs to no
    /// project.
    pub fn set_wiki_links(&self, project: Option<ProjectSlug>, existing: HashSet<NotePath>) {
        self.imp()
            .wiki_links
            .replace(Some(WikiLinks { project, existing }));
        self.restyle();
    }

    pub fn connect_wiki_link_activated(&self, callback: impl Fn(&NotePath) + 'static) {
        self.connect_closure(
            "wiki-link-activated",
            false,
            glib::closure_local!(move |_: &Self, note: String| {
                callback(&note.parse().expect("the view passes note paths"));
            }),
        );
    }

    /// The text as it is saved: in block Markdown, headings are escaped
    /// (see "Block Markdown" in the format spec).
    pub fn markdown(&self) -> String {
        let text = self.text();
        if self.full() {
            text
        } else {
            escape_headings(&text)
        }
    }

    /// Whether the view shows `text` as saved, which may differ only by
    /// headings not escaped yet and by the blank lines and spaces around it
    /// that saving drops. Otherwise showing the saved text would take away
    /// the space after a new list marker while it is being typed.
    pub fn shows(&self, text: &str) -> bool {
        let markdown = self.markdown();
        self.text() == text || markdown == text || trim_blank_lines(&markdown) == text
    }

    fn text(&self) -> String {
        let buffer = self.buffer();
        let (start, end) = buffer.bounds();
        buffer.text(&start, &end, true).into()
    }

    fn mode(&self) -> MarkdownMode {
        if self.full() {
            MarkdownMode::Full
        } else {
            MarkdownMode::Block
        }
    }

    /// Formats the whole text again, after changes, and after notes that
    /// wiki links point to were added or removed.
    fn restyle(&self) {
        self.imp().restyle_queued.set(false);
        let buffer = self.buffer();
        let (start, end) = buffer.bounds();
        // The buffer has no other tags: it has no language and no search.
        buffer.remove_all_tags(&start, &end);
        buffer.apply_tag_by_name(tags::BODY, &start, &end);
        let text = self.text();
        let offsets = char_offsets(&text);
        let styling = Styling::new(self.upcast_ref(), &text, &offsets, self.editing_cursor());
        let formatting = markdown_formatting(&text, self.mode());
        for (range, style) in &formatting.styles {
            styling.tag(&tags::name(*style), range);
        }
        // The modules after it take away what they draw otherwise, like the
        // bullets of tasks.
        let mut decorations = Decorations::collect(&styling, &formatting);
        let imp = self.imp();
        let missing = diagrams::style(&styling, &formatting, &mut decorations, &imp.diagrams);
        code_blocks::style(&styling, &formatting, &mut decorations);
        callouts::style(&styling, &formatting, &mut decorations);
        lists::style(&styling, &formatting, &mut decorations);
        tables::style(&styling, &formatting, &mut decorations);
        imp.highlighter.apply(&styling, &formatting.code_blocks);
        let revealed = decorations.revealed(styling.cursor);
        let before = imp.revealed.replace(revealed.clone());
        let left_table = decorations.left_table(&before, &revealed);
        imp.decorations.replace(decorations);

        let mut links = Vec::new();
        if let Some(wiki) = &*imp.wiki_links.borrow() {
            for link in wiki_links(&text, wiki.project.as_ref()) {
                if !link
                    .note
                    .as_ref()
                    .is_some_and(|note| wiki.existing.contains(note))
                {
                    styling.tag(tags::BROKEN_LINK, &link.span);
                }
                links.push((styling.chars(&link.span), Target::Note(link.note)));
            }
        }
        for link in &formatting.web_links {
            links.push((styling.chars(&link.range), Target::Web(link.url.clone())));
        }
        imp.links.replace(links);
        self.queue_draw();
        for request in missing {
            self.render_diagram(request);
        }
        if let Some(tidied) = left_table
            && self.is_editable()
            && !imp.loading.get()
        {
            self.queue_tidy(tidied);
        }
    }

    /// Formats the text again once the changes to it are done: one edit,
    /// like nesting a list item or undoing, often changes the buffer several
    /// times. Before the next frame is drawn, so that what was typed never
    /// shows unformatted.
    fn queue_restyle(&self) {
        if self.imp().restyle_queued.replace(true) {
            return;
        }
        glib::idle_add_local_full(
            glib::Priority::HIGH_IDLE,
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    view.restyle_if_queued();
                    glib::ControlFlow::Break
                }
            ),
        );
    }

    /// Formats the text now if that is queued, for what reads where the
    /// links and check boxes are.
    fn restyle_if_queued(&self) {
        if self.imp().restyle_queued.get() {
            self.restyle();
        }
    }

    /// Draws the diagram of `request` in the background, and formats the
    /// text again when it is ready, if the text still has it.
    fn render_diagram(&self, request: diagrams::Request) {
        let view = self.downgrade();
        glib::spawn_future_local(async move {
            let job = request.clone();
            // The renderer is young: a panic only fails the diagram.
            let image = gio::spawn_blocking(move || job.render())
                .await
                .ok()
                .flatten();
            let Some(view) = view.upgrade() else {
                return;
            };
            if view.imp().diagrams.finish(request, image) {
                view.restyle();
            }
        });
    }

    /// Where the cursor is, while the user may be editing.
    fn editing_cursor(&self) -> Option<i32> {
        (self.is_editable() && self.has_focus()).then(|| {
            let buffer = self.buffer();
            buffer.iter_at_mark(&buffer.get_insert()).offset()
        })
    }

    /// Formats the text again, when idle, if the cursor went to or away
    /// from an element that shows its Markdown while the cursor is at it.
    /// Not right away: the cursor also moves while the text changes, when
    /// tags must not. Not while text is selected either, as the text would
    /// move under the pointer.
    fn queue_reveal(&self) {
        if self.imp().reveal_queued.replace(true) {
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || {
                let imp = view.imp();
                imp.reveal_queued.set(false);
                if view.buffer().has_selection() {
                    return;
                }
                let cursor = view.editing_cursor();
                let now = imp.decorations.borrow().revealed(cursor);
                if now != *imp.revealed.borrow() {
                    view.restyle();
                }
            }
        ));
    }
}

/// The style scheme of GtkSourceView that goes with the light or `dark`
/// style of the app.
pub fn style_scheme(dark: bool) -> Option<sourceview5::StyleScheme> {
    let name = if dark { "Adwaita-dark" } else { "Adwaita" };
    sourceview5::StyleSchemeManager::default().scheme(name)
}

/// Follows light and dark style like the rest of the app. Without a scheme,
/// GtkSourceView keeps light colours.
fn set_style_scheme(view: &MarkdownView, style_manager: &adw::StyleManager) {
    let scheme = style_scheme(style_manager.is_dark());
    view.buffer()
        .downcast::<sourceview5::Buffer>()
        .expect("a source view has a source buffer")
        .set_style_scheme(scheme.as_ref());
    // Code is highlighted in the colours of the scheme.
    view.imp().highlighter.set_style_scheme(scheme.as_ref());
    view.restyle();
}

/// The character offset of each byte offset in `text` that starts a
/// character, and of its end: text buffers count characters, the core
/// counts bytes. Built once per text, so that looking up is quick.
fn char_offsets(text: &str) -> Vec<i32> {
    let mut offsets = vec![0; text.len() + 1];
    let mut count = 0;
    for (byte, _) in text.char_indices() {
        offsets[byte] = count;
        count += 1;
    }
    offsets[text.len()] = count;
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_offsets_count_characters() {
        // One, two, three and four bytes per character.
        let offsets = char_offsets("aä€𝄞b");
        let at = |byte: usize| offsets[byte];
        assert_eq!(
            [at(0), at(1), at(3), at(6), at(10), at(11)],
            [0, 1, 2, 3, 4, 5]
        );
        assert_eq!(char_offsets(""), [0]);
    }
}
