//! Mentions of contacts like `@kim` in notes and day files.

use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};

use crate::ContactSlug;
use crate::notes::parser;

/// A mention like `@kim` in a note or day file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// The mention, `@` included, as a byte range of the text.
    pub span: Range<usize>,
    /// The contact it names, whether it exists or not.
    pub contact: ContactSlug,
}

/// The mentions in `text`. Code, links, images and front matter hold
/// none, and neither does an `@` inside a word, as in a mail address.
pub fn mentions(text: &str) -> Vec<Mention> {
    let mut mentions = Vec::new();
    // How deep in code, links, images and front matter the text is.
    let mut hidden = 0;
    for (event, range) in parser(text).into_offset_iter() {
        match event {
            Event::Start(
                Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. } | Tag::MetadataBlock(_),
            ) => hidden += 1,
            Event::End(
                TagEnd::CodeBlock | TagEnd::Link | TagEnd::Image | TagEnd::MetadataBlock(_),
            ) => hidden -= 1,
            Event::Text(_) if hidden == 0 => {
                let found = text[range.clone()].match_indices('@');
                mentions.extend(found.filter_map(|(at, _)| mention_at(text, range.start + at)));
            }
            _ => {}
        }
    }
    mentions
}

/// The mention starting with the `@` at `at` in `text`, if there is one.
fn mention_at(text: &str, at: usize) -> Option<Mention> {
    if !starts_word(text, at) {
        return None;
    }
    let rest = &text[at + 1..];
    let length = rest
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    // `@kimM` and `@kim_a` are no mentions of `kim`.
    if rest[length..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return None;
    }
    // A dash at the end is punctuation, as in "ask @kim-".
    let slug = rest[..length].trim_end_matches('-');
    let contact = slug.parse().ok()?;
    Some(Mention {
        span: at..at + 1 + slug.len(),
        contact,
    })
}

/// Whether an `@` at `at` in `text` starts a word: not after a letter, a
/// digit or `_`, as in a mail address, nor after `/`, as in a web address,
/// or `\`, which escapes it.
fn starts_word(text: &str, at: usize) -> bool {
    !text[..at]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '/' | '\\'))
}

/// What is typed of the mention at `at` in `text`, as a byte range after
/// its `@` up to the end of the word, if the cursor is in one. Letters of
/// any case count, as the name may be typed rather than the slug.
pub fn typed_mention(text: &str, at: usize) -> Option<Range<usize>> {
    let is_typed = |c: char| c.is_alphanumeric() || c == '-';
    let start = text[..at]
        .rfind(|c: char| !is_typed(c))
        .filter(|&before| text[before..].starts_with('@') && starts_word(text, before))?
        + 1;
    let end = text[at..]
        .find(|c: char| !is_typed(c))
        .map_or(text.len(), |end| at + end);
    Some(start..end)
}

/// `text` with its mentions of `from` naming `to`, or `None` if it has
/// none.
pub(crate) fn renamed_mentions(text: &str, from: &ContactSlug, to: &ContactSlug) -> Option<String> {
    let mut renamed = text.to_owned();
    let mut changed = false;
    // From the end, so that the ranges before stay valid.
    for mention in mentions(text).iter().rev() {
        if mention.contact == *from {
            renamed.replace_range(mention.span.start + 1..mention.span.end, to.as_str());
            changed = true;
        }
    }
    changed.then_some(renamed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        mentions(text)
            .into_iter()
            .map(|mention| {
                assert_eq!(
                    &text[mention.span.start + 1..mention.span.end],
                    mention.contact.as_str()
                );
                &text[mention.span]
            })
            .collect()
    }

    #[test]
    fn mentions_start_words() {
        assert_eq!(found("@kim and @alex-b."), ["@kim", "@alex-b"]);
        assert_eq!(
            found("Asked (@kim), **@sam**, *@alex*, \"@lee\""),
            ["@kim", "@sam", "@alex", "@lee"]
        );
        assert_eq!(found("- [ ] call @kim\n> @sam"), ["@kim", "@sam"]);
        assert_eq!(found("ask @kim-, @kim: @kim?"), ["@kim", "@kim", "@kim"]);
        assert_eq!(
            found("| @kim | @sam |\n| --- | --- |\n| a | b |"),
            ["@kim", "@sam"]
        );
    }

    #[test]
    fn mentions_leave_out_addresses_code_and_links() {
        let text = "kim@example.com, https://example.social/@kim, \\@kim, `@kim`, \
                    [@kim](https://example.com), [[infra/@kim]], <kim@example.com>, @Kim, \
                    @kimM, @kim_a, @, @-kim, @ä\n\n```\n@kim\n```\n";
        assert!(found(text).is_empty(), "{:?}", found(text));
        assert_eq!(found("---\nwho: @kim\n---\n\n@sam"), ["@sam"]);
    }

    #[test]
    fn typed_mentions() {
        // With `^` for the cursor.
        let typed = |text: &str| {
            let at = text.find('^').expect("the text has a cursor");
            let text = text.replace('^', "");
            typed_mention(&text, at).map(|range| text[range].to_owned())
        };
        assert_eq!(typed("Ask @^").as_deref(), Some(""));
        assert_eq!(typed("@Ki^").as_deref(), Some("Ki"));
        assert_eq!(typed("(@ki^m) x").as_deref(), Some("kim"));
        assert_eq!(typed("@kim-me^").as_deref(), Some("kim-me"));
        assert_eq!(typed("@kim ^"), None);
        assert_eq!(typed("kim@exa^"), None);
        assert_eq!(typed("kim^"), None);
    }

    #[test]
    fn renaming_mentions() {
        let (from, to) = ("kim".parse().unwrap(), "kim-meyer".parse().unwrap());
        assert_eq!(
            renamed_mentions(
                "@kim, @kimberly, `@kim`, kim@example.com and @kim.",
                &from,
                &to
            )
            .as_deref(),
            Some("@kim-meyer, @kimberly, `@kim`, kim@example.com and @kim-meyer.")
        );
        assert_eq!(renamed_mentions("@sam", &from, &to), None);
    }
}
