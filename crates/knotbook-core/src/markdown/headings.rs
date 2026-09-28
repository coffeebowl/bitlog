//! Headings, which block Markdown has none of, so they are escaped there.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};

use super::OPTIONS;

/// The headings in `text`, as the byte ranges of their first lines. Block
/// Markdown has no headings, so each of them is a problem there.
pub fn heading_lines(text: &str) -> Vec<Range<usize>> {
    headings(text)
        .map(|range| {
            let line_end = text[range.clone()]
                .find('\n')
                .map_or(range.end, |end| range.start + end);
            range.start..line_end
        })
        .collect()
}

/// `text` with every heading escaped, so that it reads as plain text:
/// `\# Title`, or `\---` below the text of a setext heading.
pub fn escape_headings(text: &str) -> String {
    let mut escaped = text.to_owned();
    let positions: Vec<usize> = headings(text)
        .map(|range| {
            let heading = &text[range.clone()];
            // Setext headings span two lines, ATX headings one.
            let (offset, marks) = match heading.trim_end().rfind('\n') {
                Some(line_start) => (line_start, ['-', '=']),
                None => (0, ['#', '#']),
            };
            let mark = heading[offset..]
                .find(marks)
                .expect("a heading has its marks");
            range.start + offset + mark
        })
        .collect();
    // From the back, so that earlier positions stay valid.
    for position in positions.into_iter().rev() {
        escaped.insert(position, '\\');
    }
    escaped
}

fn headings(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    parser(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::Heading { .. })).then_some(range)
        })
}

fn parser(text: &str) -> Parser<'_> {
    Parser::new_ext(text, OPTIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_all_kinds_of_headings() {
        let text = "# Title\n\nText\n\n## Old notes {#zz99}\n\n> ### Quoted\n\n\
                    - # In a list\n\nSetext\n---\n\nOther\n===\n\n\
                    ```\n# Code\n```\n\n#tag and \\# escaped\n";
        let lines: Vec<&str> = heading_lines(text)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(
            lines,
            [
                "# Title",
                "## Old notes {#zz99}",
                "### Quoted",
                "# In a list",
                "Setext",
                "Other"
            ]
        );
        let escaped = escape_headings(text);
        assert_eq!(
            escaped,
            "\\# Title\n\nText\n\n\\## Old notes {#zz99}\n\n> \\### Quoted\n\n\
             - \\# In a list\n\nSetext\n\\---\n\nOther\n\\===\n\n\
             ```\n# Code\n```\n\n#tag and \\# escaped\n"
        );
        assert!(heading_lines(&escaped).is_empty());
        assert_eq!(escape_headings(&escaped), escaped);
    }
}
