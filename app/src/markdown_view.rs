use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::translate::IntoGlib;
use gtk::{glib, pango};
use knotbook_core::{MarkdownMode, MarkdownStyle, markdown_styles};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;

/// How much Markdown syntax is dimmed, as the alpha of the text colour.
const MARKUP_ALPHA: f32 = 0.45;
const HEADING_SCALES: [f64; 6] = [1.6, 1.4, 1.25, 1.1, 1.0, 1.0];

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
    }

    #[glib::derived_properties]
    impl ObjectImpl for MarkdownView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            view.set_editable(false);
            view.set_cursor_visible(false);
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            view.add_css_class("markdown-view");
            create_tags(&view.buffer());

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
    pub struct MarkdownView(ObjectSubclass<imp::MarkdownView>)
        @extends sourceview5::View, gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Scrollable;
}

impl MarkdownView {
    pub fn set_markdown(&self, text: &str) {
        let buffer = self.buffer();
        buffer.set_text(text);
        let mode = if self.full() {
            MarkdownMode::Full
        } else {
            MarkdownMode::Block
        };
        for (range, style) in markdown_styles(text, mode) {
            let start = buffer.iter_at_offset(char_offset(text, range.start));
            let end = buffer.iter_at_offset(char_offset(text, range.end));
            buffer.apply_tag_by_name(&tag_name(style), &start, &end);
        }
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
