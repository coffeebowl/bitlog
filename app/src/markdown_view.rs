use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::translate::IntoGlib;
use gtk::{gdk, glib, pango};
use knotbook_core::{MarkdownMode, MarkdownStyle, escape_headings, markdown_styles};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

/// How much Markdown syntax is dimmed, as the alpha of the text colour.
const MARKUP_ALPHA: f32 = 0.45;
const HEADING_SCALES: [f64; 6] = [1.6, 1.4, 1.25, 1.1, 1.0, 1.0];
/// The actions that change the text, only enabled while it is editable.
const EDIT_ACTIONS: [&str; 3] = ["markdown.bold", "markdown.italic", "markdown.code"];

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MarkdownView)]
    pub struct MarkdownView {
        /// Whether headings are formatted. Block texts and day notes keep
        /// them as text, project notes format them.
        #[property(get, set)]
        pub full: Cell<bool>,
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
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MarkdownView {
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
                move |_| view.restyle()
            ));
            view.connect_full_notify(|view| view.restyle());

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
    }

    impl MarkdownView {
        /// Dims Markdown syntax relative to the text colour of the theme.
        fn update_markup_color(&self) {
            let view = self.obj();
            let color = view.color();
            let markup = view
                .buffer()
                .tag_table()
                .lookup("markup")
                .expect("the tags are created on construction");
            markup.set_foreground_rgba(Some(&color.with_alpha(color.alpha() * MARKUP_ALPHA)));
        }
    }

    impl TextViewImpl for MarkdownView {}
    impl ViewImpl for MarkdownView {}
}

glib::wrapper! {
    /// Markdown with live formatting: the syntax stays visible, but dimmed.
    ///
    /// Read-only unless made editable. When editable, Ctrl+B, Ctrl+I and
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
        buffer.begin_irreversible_action();
        buffer.set_text(text);
        buffer.end_irreversible_action();
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

    fn text(&self) -> String {
        let buffer = self.buffer();
        let (start, end) = buffer.bounds();
        buffer.text(&start, &end, true).into()
    }

    /// Formats the whole text again, after every change.
    fn restyle(&self) {
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
