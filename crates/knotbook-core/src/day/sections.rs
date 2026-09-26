//! Splits the Markdown of a day file into the day note and block sections.

use std::collections::HashSet;

use chrono::NaiveDate;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag};

use super::DayWarning;
use crate::BlockId;

pub(super) struct Body<'a> {
    pub note: &'a str,
    pub sections: Vec<Section<'a>>,
    pub warnings: Vec<DayWarning>,
}

pub(super) struct Section<'a> {
    pub id: BlockId,
    pub title: &'a str,
    pub text: &'a str,
}

/// A block heading: where its line starts and ends, its title and its block.
struct Boundary<'a> {
    start: usize,
    end: usize,
    title: &'a str,
    id: BlockId,
}

pub(super) fn split<'a>(body: &'a str, date: NaiveDate, ids: &[BlockId]) -> Body<'a> {
    let (boundaries, warnings) = find_boundaries(body, ids);

    let before_blocks = &body[..boundaries.first().map_or(body.len(), |b| b.start)];
    let note = trim_blank_lines(strip_date_heading(before_blocks, date));

    let sections = boundaries
        .iter()
        .enumerate()
        .map(|(i, boundary)| {
            let text_end = boundaries.get(i + 1).map_or(body.len(), |next| next.start);
            Section {
                id: boundary.id.clone(),
                title: boundary.title,
                text: trim_blank_lines(&body[boundary.end..text_end]),
            }
        })
        .collect();

    Body {
        note,
        sections,
        warnings,
    }
}

/// Finds the level 2 headings that start a block: at the start of a line,
/// outside code blocks and quotes, ending in the marker of a known block.
fn find_boundaries<'a>(body: &'a str, ids: &[BlockId]) -> (Vec<Boundary<'a>>, Vec<DayWarning>) {
    let mut boundaries = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();

    for (event, range) in Parser::new(body).into_offset_iter() {
        let Event::Start(Tag::Heading {
            level: HeadingLevel::H2,
            ..
        }) = event
        else {
            continue;
        };
        let at_line_start = range.start == 0 || body[..range.start].ends_with('\n');
        let line = body[range.start..].lines().next().unwrap_or_default();
        let Some((title, marker)) = parse_heading(line).filter(|_| at_line_start) else {
            continue;
        };

        match ids.iter().find(|id| id.as_str() == marker) {
            Some(id) if seen.insert(id) => boundaries.push(Boundary {
                start: range.start,
                end: range.start + line.len(),
                title,
                id: id.clone(),
            }),
            Some(id) => warnings.push(DayWarning::DuplicateMarker { id: id.clone() }),
            None => warnings.push(DayWarning::UnknownMarker {
                id: marker.to_owned(),
            }),
        }
    }
    (boundaries, warnings)
}

/// Splits `## Title {#id}` into title and id.
fn parse_heading(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("## ")?.trim_end();
    let (title, marker) = rest.rsplit_once("{#")?;
    let id = marker.strip_suffix('}')?;
    (title.is_empty() || title.ends_with(' ')).then_some((title.trim(), id))
}

fn strip_date_heading(text: &str, date: NaiveDate) -> &str {
    let heading = format!("# {}", date.format("%Y-%m-%d"));
    let text = text.trim_start_matches(['\n', '\r']);
    match text.split_once('\n') {
        Some((first, rest)) if first.trim_end() == heading => rest,
        None if text.trim_end() == heading => "",
        _ => text,
    }
}

/// Removes the blank lines around a text. They belong to the layout of the
/// file, not to the text.
fn trim_blank_lines(text: &str) -> &str {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        start += line.len();
    }
    text[start..].trim_end()
}
