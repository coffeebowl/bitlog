//! Syntax highlighting for fenced code blocks, by syntect with the grammars
//! of bat, in the colours of the GtkSourceView style scheme. Languages
//! syntect does not know, GtkSourceView highlights in a buffer of its own,
//! whose looks the Markdown view copies.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{LazyLock, Once};

use bitlog_core::CodeBlock;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, pango};
use sourceview5::prelude::*;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use super::styling::Styling;
use super::tags;

/// How many highlighted blocks are kept, so that typing elsewhere does not
/// highlight them again.
const CACHE_SIZE: usize = 64;

/// Which style of the scheme each TextMate scope takes. The most specific
/// scope wins; none keeps the colour of the text.
const SCOPE_STYLES: [(&str, Option<&str>); 21] = [
    ("comment", Some("def:comment")),
    ("string", Some("def:string")),
    ("constant", Some("def:constant")),
    ("constant.numeric", Some("def:number")),
    ("constant.character.escape", Some("def:special-char")),
    ("support.constant", Some("def:constant")),
    ("variable.language", Some("def:constant")),
    ("keyword", Some("def:statement")),
    ("storage", Some("def:statement")),
    ("keyword.operator", None),
    ("entity.name.function", Some("def:function")),
    ("support.function", Some("def:function")),
    ("variable.function", Some("def:function")),
    ("entity.name.type", Some("def:type")),
    ("entity.name.class", Some("def:type")),
    ("entity.other.inherited-class", Some("def:type")),
    ("support.type", Some("def:type")),
    ("support.class", Some("def:type")),
    ("entity.name.tag", Some("def:identifier")),
    ("entity.other.attribute-name", Some("def:constant")),
    ("meta.preprocessor", Some("def:preprocessor")),
];

/// Marks text in the colour of the text, which no scheme has.
const TEXT_COLOR: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

/// Loaded once, when first needed.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// Loads the grammars in the background, which takes a moment, so that
/// the first code block need not wait for them.
pub fn preload() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| LazyLock::force(&SYNTAXES));
    });
}

/// How a part of the code looks, as the style scheme says.
#[derive(Debug, Clone, Default, PartialEq)]
struct Look {
    foreground: Option<gdk::RGBA>,
    weight: Option<i32>,
    style: Option<pango::Style>,
}

impl Look {
    /// What `tags` in ascending priority make of the text.
    fn of(tags: &[gtk::TextTag]) -> Self {
        let mut look = Self::default();
        for tag in tags {
            if tag.is_foreground_set() {
                look.foreground = tag.foreground_rgba();
            }
            if tag.is_weight_set() {
                look.weight = Some(tag.weight());
            }
            if tag.is_style_set() {
                look.style = Some(tag.style());
            }
        }
        look
    }

    fn from_syntect(style: syntect::highlighting::Style) -> Self {
        let color = style.foreground;
        let channel = |value: u8| f32::from(value) / 255.0;
        Self {
            foreground: (color != TEXT_COLOR)
                .then(|| gdk::RGBA::new(channel(color.r), channel(color.g), channel(color.b), 1.0)),
            weight: style
                .font_style
                .contains(FontStyle::BOLD)
                .then(|| pango::Weight::Bold.into_glib()),
            style: style
                .font_style
                .contains(FontStyle::ITALIC)
                .then_some(pango::Style::Italic),
        }
    }

    /// Tells tags apart, so that each look has one.
    fn tag_name(&self) -> String {
        let color = self.foreground.map(|color| color.to_str());
        format!("syntax {color:?} {:?} {:?}", self.weight, self.style)
    }

    fn tag(&self) -> gtk::TextTag {
        let tag = gtk::TextTag::new(Some(&self.tag_name()));
        tag.set_foreground_rgba(self.foreground.as_ref());
        if let Some(weight) = self.weight {
            tag.set_weight(weight);
        }
        if let Some(style) = self.style {
            tag.set_style(style);
        }
        tag
    }
}

/// A part of the code, as character offsets into it, and how it looks.
type Span = (Range<i32>, Look);

/// Highlighted code by language and code.
type Cache = HashMap<(String, String), Rc<[Span]>>;

#[derive(Debug, Default)]
pub struct CodeHighlighter {
    /// The colours of the scheme, for syntect.
    theme: RefCell<Theme>,
    /// A buffer for each language as code blocks name it, or none if
    /// GtkSourceView does not know the language.
    buffers: RefCell<HashMap<String, Option<sourceview5::Buffer>>>,
    scheme: RefCell<Option<sourceview5::StyleScheme>>,
    cache: RefCell<Cache>,
}

impl CodeHighlighter {
    pub fn set_style_scheme(&self, scheme: Option<&sourceview5::StyleScheme>) {
        for buffer in self.buffers.borrow().values().flatten() {
            buffer.set_style_scheme(scheme);
        }
        self.theme.replace(scheme.map(theme).unwrap_or_default());
        self.scheme.replace(scheme.cloned());
        self.cache.borrow_mut().clear();
    }

    /// Highlights the code of `blocks` that name a language.
    pub(super) fn apply(&self, styling: &Styling, blocks: &[CodeBlock]) {
        for block in blocks.iter().filter(|block| !block.language.is_empty()) {
            let start = styling.offset(block.code.start);
            let code = &styling.text[block.code.clone()];
            for (range, look) in self.highlight(&block.language, code).iter() {
                let name = look.tag_name();
                tags::get_or_add(&styling.buffer, &name, || look.tag());
                tags::apply(
                    &styling.buffer,
                    &name,
                    start + range.start..start + range.end,
                );
            }
        }
    }

    /// The parts of `code` that stand out, none if `language` is unknown.
    fn highlight(&self, language: &str, code: &str) -> Rc<[Span]> {
        let key = (language.to_owned(), code.to_owned());
        if let Some(spans) = self.cache.borrow().get(&key) {
            return spans.clone();
        }
        let spans: Rc<[Span]> = if let Some(spans) = self.syntect_spans(language, code) {
            spans.into()
        } else if let Some(buffer) = self.buffer(language) {
            buffer_spans(&buffer, code).into()
        } else {
            Rc::new([])
        };
        let mut cache = self.cache.borrow_mut();
        if cache.len() >= CACHE_SIZE {
            cache.clear();
        }
        cache.insert(key, spans.clone());
        spans
    }

    /// Highlights `code` by syntect, if it knows `language`.
    fn syntect_spans(&self, language: &str, code: &str) -> Option<Vec<Span>> {
        let syntaxes = &*SYNTAXES;
        let language = match language.to_lowercase().as_str() {
            "console" | "shell" | "zsh" => "bash".to_owned(),
            language => language.to_owned(),
        };
        let syntax = syntaxes.find_syntax_by_token(&language)?;
        let theme = self.theme.borrow();
        let mut highlighter = HighlightLines::new(syntax, &theme);
        let mut spans = Vec::new();
        let mut offset = 0;
        for line in LinesWithEndings::from(code) {
            // Only broken grammars fail, which then leave the rest as it is.
            let Ok(parts) = highlighter.highlight_line(line, syntaxes) else {
                break;
            };
            for (style, part) in parts {
                let length = i32::try_from(part.chars().count()).unwrap_or(i32::MAX);
                let look = Look::from_syntect(style);
                if look != Look::default() {
                    spans.push((offset..offset + length, look));
                }
                offset += length;
            }
        }
        Some(spans)
    }

    fn buffer(&self, language: &str) -> Option<sourceview5::Buffer> {
        self.buffers
            .borrow_mut()
            .entry(language.to_owned())
            .or_insert_with(|| {
                let buffer = sourceview5::Buffer::with_language(&find_language(language)?);
                buffer.set_style_scheme(self.scheme.borrow().as_ref());
                Some(buffer)
            })
            .clone()
    }
}

/// A syntect theme in the colours of `scheme`.
fn theme(scheme: &sourceview5::StyleScheme) -> Theme {
    let items = SCOPE_STYLES.iter().filter_map(|(scope, style)| {
        let style = match style {
            Some(style) => {
                let style = scheme.style(style)?;
                let color = style
                    .foreground()
                    .filter(|_| style.is_foreground_set())
                    .and_then(|color| gdk::RGBA::parse(color.as_str()).ok())?;
                let channel = |value: f32| (value * 255.0).round() as u8;
                let mut font_style = FontStyle::empty();
                font_style.set(FontStyle::BOLD, style.is_bold_set() && style.is_bold());
                font_style.set(
                    FontStyle::ITALIC,
                    style.is_italic_set() && style.is_italic(),
                );
                StyleModifier {
                    foreground: Some(Color {
                        r: channel(color.red()),
                        g: channel(color.green()),
                        b: channel(color.blue()),
                        a: 255,
                    }),
                    background: None,
                    font_style: Some(font_style),
                }
            }
            None => StyleModifier {
                foreground: Some(TEXT_COLOR),
                background: None,
                font_style: Some(FontStyle::empty()),
            },
        };
        Some(ThemeItem {
            scope: ScopeSelectors::from_str(scope).ok()?,
            style,
        })
    });
    Theme {
        settings: ThemeSettings {
            foreground: Some(TEXT_COLOR),
            ..ThemeSettings::default()
        },
        scopes: items.collect(),
        ..Theme::default()
    }
}

/// Highlights `code` in `buffer`, right away instead of when idle.
fn buffer_spans(buffer: &sourceview5::Buffer, code: &str) -> Vec<Span> {
    buffer.set_text(code);
    let (mut iter, end) = buffer.bounds();
    buffer.ensure_highlight(&iter, &end);
    let mut spans = Vec::new();
    while !iter.is_end() {
        let mut next = iter;
        next.forward_to_tag_toggle(None::<&gtk::TextTag>);
        let look = Look::of(&iter.tags());
        if look != Look::default() {
            spans.push((iter.offset()..next.offset(), look));
        }
        iter = next;
    }
    spans
}

/// The language `name` stands for in GtkSourceView, by its ID or file
/// extension, as in `python3` or `py`.
fn find_language(name: &str) -> Option<sourceview5::Language> {
    let name = name.to_lowercase();
    let name = match name.as_str() {
        "bash" | "console" | "shell" | "zsh" => "sh",
        "dockerfile" => "docker",
        name => name,
    };
    let manager = sourceview5::LanguageManager::default();
    manager
        .language(name)
        .or_else(|| manager.guess_language(Some(format!("code.{name}")), None))
}
