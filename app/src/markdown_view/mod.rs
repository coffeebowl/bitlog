mod callouts;
mod check_boxes;
mod code_blocks;
mod code_highlight;
mod decorations;
mod diagrams;
mod lists;
mod styling;
mod tables;
mod tags;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::SignalHandlerId;
use glib::subclass::Signal;
use gtk::{gdk, gio, glib};
use knotbook_core::{
    MarkdownMode, NotePath, ProjectSlug, escape_headings, markdown_formatting, wiki_links,
};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

use self::check_boxes::CheckBox;
use self::code_highlight::CodeHighlighter;
use self::decorations::Decorations;
use self::diagrams::Diagrams;
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

/// Where a link leads.
#[derive(Debug, Clone)]
enum Target {
    /// A note of the vault, or none if the wiki link points nowhere.
    Note(Option<NotePath>),
    /// A web page or mail address, which another app opens.
    Web(String),
}

/// Tells the wiki links of a project note apart, see
/// `MarkdownView::set_wiki_links`.
#[derive(Debug)]
pub struct WikiLinks {
    project: ProjectSlug,
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
        /// Set for project notes, whose wiki links can be followed.
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
        /// Whether the text is to be formatted again when idle, as the
        /// view changed its width.
        pub restyle_queued: Cell<bool>,
        pub(super) diagrams: Diagrams,
        pub highlighter: CodeHighlighter,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MarkdownView {
        const NAME: &'static str = "KnotbookMarkdownView";
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
                    view.restyle();
                    if !view.imp().loading.get() {
                        view.emit_by_name::<()>("edited", &[]);
                    }
                }
            ));
            view.connect_full_notify(|view| view.restyle());
            view.follow_links_on_click();
            view.toggle_check_boxes_on_click();
            view.edit_lists_by_keys();

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

        /// Makes room for diagrams again as they fit the new width.
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let view = self.obj();
            if self.decorations.borrow().misfit(view.upcast_ref())
                && !self.restyle_queued.replace(true)
            {
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    view,
                    move || {
                        view.imp().restyle_queued.set(false);
                        view.restyle();
                    }
                ));
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
    /// a language, or diagrams when they are in Mermaid. Tables are grids
    /// and rules are lines, but show their Markdown while the cursor is in
    /// them, with the columns lined up.
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

    /// Makes the wiki links of this note of `project` followable, and dims
    /// those to notes missing from `existing`.
    pub fn set_wiki_links(&self, project: ProjectSlug, existing: HashSet<NotePath>) {
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
    /// headings not escaped yet.
    pub fn shows(&self, text: &str) -> bool {
        self.text() == text || self.markdown() == text
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

    /// Formats the whole text again, after every change, and after notes
    /// that wiki links point to were added or removed.
    fn restyle(&self) {
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
        imp.revealed.replace(decorations.revealed(styling.cursor));
        imp.decorations.replace(decorations);

        let mut links = Vec::new();
        if let Some(wiki) = &*imp.wiki_links.borrow() {
            for link in wiki_links(&text, Some(&wiki.project)) {
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

    /// Where the link at `iter` leads, if there is one.
    fn link_at(&self, iter: &gtk::TextIter) -> Option<Target> {
        self.imp()
            .links
            .borrow()
            .iter()
            .find(|(range, _)| range.contains(&iter.offset()))
            .map(|(_, target)| target.clone())
    }

    /// Follows the link at `iter`, if there is one.
    fn follow_link_at(&self, iter: &gtk::TextIter) {
        if let Some(target) = self.link_at(iter) {
            self.follow_link(target);
        }
    }

    /// Opens a note of the vault, or another app for a web page or mail
    /// address.
    fn follow_link(&self, target: Target) {
        match target {
            Target::Note(Some(note)) => {
                self.emit_by_name::<()>("wiki-link-activated", &[&note.to_string()]);
            }
            // Points nowhere, like `[[a/b/c]]`.
            Target::Note(None) => self.error_bell(),
            Target::Web(url) => {
                let window = self.root().and_downcast::<gtk::Window>();
                gtk::UriLauncher::new(&url).launch(
                    window.as_ref(),
                    None::<&gio::Cancellable>,
                    |_| {},
                );
            }
        }
    }

    /// The text at `x`, `y` in widget coordinates, if there is any.
    fn iter_at(&self, x: f64, y: f64) -> Option<gtk::TextIter> {
        let (x, y) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        self.iter_at_location(x, y)
    }

    /// A click on a link follows it, as in the "Hypertext" demo of GTK, and
    /// the pointer shows where that is possible.
    fn follow_links_on_click(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        // The link under the pointer as the button goes down: the text may
        // move before it goes up, as the cursor reveals the markup.
        let pressed = Rc::new(RefCell::new(None));
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[strong]
            pressed,
            move |_, _, x, y| {
                let link = view.iter_at(x, y).and_then(|iter| view.link_at(&iter));
                pressed.replace(link);
            }
        ));
        click.connect_released(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, presses, _, _| {
                let link = pressed.take();
                // Selecting text is no click on a link.
                if presses != 1 || view.buffer().has_selection() {
                    return;
                }
                if let Some(target) = link {
                    view.follow_link(target);
                }
            }
        ));
        self.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, x, y| {
                let on_link = view
                    .iter_at(x, y)
                    .is_some_and(|iter| view.link_at(&iter).is_some());
                let on_check_box = view.is_editable() && view.check_box_at(x, y).is_some();
                let pointer = on_link || on_check_box;
                view.set_cursor_from_name(Some(if pointer { "pointer" } else { "text" }));
            }
        ));
        self.add_controller(motion);
    }

    /// What a click on the check box at `x`, `y` in widget coordinates
    /// replaces, and with what, if there is one.
    fn check_box_at(&self, x: f64, y: f64) -> Option<(Range<i32>, String)> {
        let (x, y) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let decorations = self.imp().decorations.borrow();
        decorations
            .check_box_at(self.upcast_ref(), x, y)
            .map(CheckBox::toggle)
    }

    /// A click on a check box checks or unchecks it, before the view would
    /// move the cursor there.
    fn toggle_check_boxes_on_click(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |click, _, x, y| {
                if !view.is_editable() {
                    return;
                }
                let Some((range, replacement)) = view.check_box_at(x, y) else {
                    return;
                };
                click.set_state(gtk::EventSequenceState::Claimed);
                let buffer = view.buffer();
                let mut start = buffer.iter_at_offset(range.start);
                let mut end = buffer.iter_at_offset(range.end);
                buffer.begin_user_action();
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &replacement);
                buffer.end_user_action();
            }
        ));
        self.add_controller(click);
    }

    /// In list items, Tab nests the item deeper, Shift+Tab less deep, and
    /// Enter starts the next item, before the view would handle the keys.
    /// Shift+Enter still only breaks the line.
    fn edit_lists_by_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let others = gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK;
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                if !view.is_editable() || modifiers.intersects(others) {
                    return glib::Propagation::Proceed;
                }
                let is_handled = match key {
                    gdk::Key::Tab => lists::nest(view.upcast_ref(), view.mode(), !shift),
                    gdk::Key::ISO_Left_Tab => lists::nest(view.upcast_ref(), view.mode(), false),
                    gdk::Key::Return | gdk::Key::KP_Enter if !shift => {
                        lists::continue_item(view.upcast_ref(), view.mode())
                    }
                    _ => false,
                };
                if is_handled {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
    }

    /// Puts `marker` around the selection, or at the cursor, and selects
    /// what it wraps. Takes it away instead if it is there already.
    fn toggle_marker(&self, marker: &str) {
        let buffer = self.buffer();
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            (cursor, cursor)
        });
        let (start, end) = (start.offset(), end.offset());
        let length = i32::try_from(marker.chars().count()).expect("markers are short");
        let mark = marker.chars().next().expect("markers are not empty");
        let is_mark = |iter: &gtk::TextIter| iter.char() == mark;
        // How many marker characters there are on each side, as in `***`.
        let mut before = 0;
        let mut iter = buffer.iter_at_offset(start);
        while iter.backward_char() && is_mark(&iter) {
            before += 1;
        }
        let mut after = 0;
        let mut iter = buffer.iter_at_offset(end);
        // The end of the text has no character, so the loop stops there.
        while is_mark(&iter) {
            after += 1;
            iter.forward_char();
        }
        let run = before.min(after);
        let is_wrapped = match marker {
            // In `***both***`, one star is italic and two are bold.
            "*" => run % 2 == 1,
            "**" => run >= 2,
            _ => run >= 1,
        };

        buffer.begin_user_action();
        let (start, end) = if is_wrapped {
            buffer.delete(
                &mut buffer.iter_at_offset(end),
                &mut buffer.iter_at_offset(end + length),
            );
            buffer.delete(
                &mut buffer.iter_at_offset(start - length),
                &mut buffer.iter_at_offset(start),
            );
            (start - length, end - length)
        } else {
            buffer.insert(&mut buffer.iter_at_offset(end), marker);
            buffer.insert(&mut buffer.iter_at_offset(start), marker);
            (start + length, end + length)
        };
        buffer.select_range(&buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
        buffer.end_user_action();
    }
}

/// Follows light and dark style like the rest of the app. Without a scheme,
/// GtkSourceView keeps light colours.
fn set_style_scheme(view: &MarkdownView, style_manager: &adw::StyleManager) {
    let name = if style_manager.is_dark() {
        "Adwaita-dark"
    } else {
        "Adwaita"
    };
    let scheme = sourceview5::StyleSchemeManager::default().scheme(name);
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
