use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::subclass::Signal;
use glib::translate::IntoGlib;
use gtk::{gdk, glib, pango};
use knotbook_core::{
    MarkdownMode, MarkdownStyle, NotePath, ProjectSlug, WikiLink, escape_headings, markdown_styles,
    wiki_links,
};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

/// How much Markdown syntax is dimmed, as the alpha of the text colour.
const MARKUP_ALPHA: f32 = 0.45;
const HEADING_SCALES: [f64; 6] = [1.6, 1.4, 1.25, 1.1, 1.0, 1.0];
/// The actions that change the text, only enabled while it is editable.
const EDIT_ACTIONS: [&str; 3] = ["markdown.bold", "markdown.italic", "markdown.code"];
/// Dims wiki links to notes that do not exist.
const BROKEN_LINK_TAG: &str = "broken-link";

/// Tells the wiki links of a project note apart, see
/// `MarkdownView::set_wiki_links`.
pub struct WikiLinks {
    project: ProjectSlug,
    exists: Box<dyn Fn(&NotePath) -> bool>,
}

impl std::fmt::Debug for WikiLinks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WikiLinks")
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
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
            });
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            view.add_css_class("markdown-view");
            create_tags(&view.buffer());
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
                        move || view.imp().update_markup_color()
                    ));
                }
            );
            style_manager.connect_dark_notify(on_change.clone());
            style_manager.connect_high_contrast_notify(on_change);
        }
    }

    impl WidgetImpl for MarkdownView {
        fn map(&self) {
            self.parent_map();
            self.update_markup_color();
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
            snapshot.append_layout(&layout, &color.with_alpha(color.alpha() * MARKUP_ALPHA));
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

        /// Dims Markdown syntax relative to the text colour of the theme.
        fn update_markup_color(&self) {
            let view = self.obj();
            let color = view.color();
            let markup = view
                .buffer()
                .tag_table()
                .lookup("markup")
                .expect("the tags are created on construction");
            let dimmed = color.with_alpha(color.alpha() * MARKUP_ALPHA);
            markup.set_foreground_rgba(Some(&dimmed));
            view.buffer()
                .tag_table()
                .lookup(BROKEN_LINK_TAG)
                .expect("the tags are created on construction")
                .set_foreground_rgba(Some(&dimmed));
        }
    }

    impl TextViewImpl for MarkdownView {}
    impl ViewImpl for MarkdownView {}
}

glib::wrapper! {
    /// Markdown with live formatting: the syntax stays visible, but dimmed.
    ///
    /// Read-only unless made editable, with a placeholder while empty. When editable, Ctrl+B, Ctrl+I and
    /// Ctrl+E make the selection bold, italic or code, or undo that.
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
    /// those for which `exists` is false.
    pub fn set_wiki_links(
        &self,
        project: ProjectSlug,
        exists: impl Fn(&NotePath) -> bool + 'static,
    ) {
        self.imp().wiki_links.replace(Some(WikiLinks {
            project,
            exists: Box::new(exists),
        }));
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

    /// Formats the whole text again, after every change, and after notes
    /// that wiki links point to were added or removed.
    pub fn restyle(&self) {
        let buffer = self.buffer();
        let (start, end) = buffer.bounds();
        // The buffer has no other tags: it has no language and no search.
        buffer.remove_all_tags(&start, &end);
        let text = self.text();
        let mode = if self.full() {
            MarkdownMode::Full
        } else {
            MarkdownMode::Block
        };
        for (range, style) in markdown_styles(&text, mode) {
            let start = buffer.iter_at_offset(char_offset(&text, range.start));
            let end = buffer.iter_at_offset(char_offset(&text, range.end));
            buffer.apply_tag_by_name(&tag_name(style), &start, &end);
        }
        if let Some(links) = &*self.imp().wiki_links.borrow() {
            for link in wiki_links(&text, &links.project) {
                if !link.note.as_ref().is_some_and(|note| (links.exists)(note)) {
                    let start = buffer.iter_at_offset(char_offset(&text, link.span.start));
                    let end = buffer.iter_at_offset(char_offset(&text, link.span.end));
                    buffer.apply_tag_by_name(BROKEN_LINK_TAG, &start, &end);
                }
            }
        }
    }

    /// The wiki link at `iter`, if there is one.
    fn wiki_link_at(&self, iter: &gtk::TextIter) -> Option<WikiLink> {
        let links = self.imp().wiki_links.borrow();
        let links = links.as_ref()?;
        let text = self.text();
        let offset = usize::try_from(iter.offset()).expect("offsets are not negative");
        let byte = text
            .char_indices()
            .nth(offset)
            .map_or(text.len(), |(byte, _)| byte);
        wiki_links(&text, &links.project)
            .into_iter()
            .find(|link| link.span.contains(&byte))
    }

    /// Follows the wiki link at `iter`, if there is one.
    fn follow_link_at(&self, iter: &gtk::TextIter) {
        match self.wiki_link_at(iter).map(|link| link.note) {
            Some(Some(note)) => {
                self.emit_by_name::<()>("wiki-link-activated", &[&note.to_string()]);
            }
            // Points nowhere, like `[[a/b/c]]`.
            Some(None) => self.error_bell(),
            None => {}
        }
    }

    /// The text at `x`, `y` in widget coordinates, if there is any.
    fn iter_at(&self, x: f64, y: f64) -> Option<gtk::TextIter> {
        let (x, y) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        self.iter_at_location(x, y)
    }

    /// A click on a wiki link follows it, as in the "Hypertext" demo of
    /// GTK, and the pointer shows where that is possible.
    fn follow_links_on_click(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        click.connect_released(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, presses, x, y| {
                // Selecting text is no click on a link.
                if presses != 1 || view.buffer().has_selection() {
                    return;
                }
                if let Some(iter) = view.iter_at(x, y) {
                    view.follow_link_at(&iter);
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
                    .is_some_and(|iter| view.wiki_link_at(&iter).is_some());
                view.set_cursor_from_name(Some(if on_link { "pointer" } else { "text" }));
            }
        ));
        self.add_controller(motion);
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
}

/// Creates a tag for every style. The markup tag comes last, so that it wins.
fn create_tags(buffer: &gtk::TextBuffer) {
    let monospace = || gtk::TextTag::builder().family("monospace");
    let tags = [
        gtk::TextTag::builder().weight(pango::Weight::Bold.into_glib()),
        gtk::TextTag::builder().style(pango::Style::Italic),
        gtk::TextTag::builder().strikethrough(true),
        monospace(),
        monospace(),
        gtk::TextTag::builder().underline(pango::Underline::Single),
        gtk::TextTag::builder()
            .style(pango::Style::Italic)
            .left_margin(12),
    ];
    let styles = [
        MarkdownStyle::Strong,
        MarkdownStyle::Emphasis,
        MarkdownStyle::Strikethrough,
        MarkdownStyle::Code,
        MarkdownStyle::CodeBlock,
        MarkdownStyle::Link,
        MarkdownStyle::Quote,
    ];
    let table = buffer.tag_table();
    for (builder, style) in tags.into_iter().zip(styles) {
        table.add(&builder.name(tag_name(style)).build());
    }
    for (level, scale) in (1..).zip(HEADING_SCALES) {
        let tag = gtk::TextTag::builder()
            .name(tag_name(MarkdownStyle::Heading(level)))
            .weight(pango::Weight::Bold.into_glib())
            .scale(scale)
            .build();
        table.add(&tag);
    }
    table.add(&gtk::TextTag::new(Some(BROKEN_LINK_TAG)));
    table.add(&gtk::TextTag::new(Some(&tag_name(MarkdownStyle::Markup))));
}

fn tag_name(style: MarkdownStyle) -> String {
    match style {
        MarkdownStyle::Strong => "strong".to_owned(),
        MarkdownStyle::Emphasis => "emphasis".to_owned(),
        MarkdownStyle::Strikethrough => "strikethrough".to_owned(),
        MarkdownStyle::Code => "code".to_owned(),
        MarkdownStyle::CodeBlock => "code-block".to_owned(),
        MarkdownStyle::Link => "link".to_owned(),
        MarkdownStyle::Quote => "quote".to_owned(),
        MarkdownStyle::Heading(level) => format!("heading-{level}"),
        MarkdownStyle::Markup => "markup".to_owned(),
    }
}

/// Text buffers count characters, the core counts bytes.
fn char_offset(text: &str, byte: usize) -> i32 {
    text[..byte]
        .chars()
        .count()
        .try_into()
        .expect("a note fits into a text buffer")
}
