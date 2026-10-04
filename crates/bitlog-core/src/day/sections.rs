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
    /// The id of the block, as the marker of its heading gives it.
    pub id: &'a str,
    pub title: &'a str,
    pub text: &'a str,
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
                id: boundary.marker,
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

/// A level 2 heading with an ID marker: where its line starts and ends, its
/// title and the marker.
struct MarkerHeading<'a> {
    start: usize,
    end: usize,
    title: &'a str,
    marker: &'a str,
}

/// Finds the level 2 headings with an ID marker at the start of a line,
/// outside code blocks and quotes.
fn marker_headings(text: &str) -> Vec<MarkerHeading<'_>> {
    let mut headings = Vec::new();
    for (event, range) in Parser::new(text).into_offset_iter() {
        let Event::Start(Tag::Heading {
            level: HeadingLevel::H2,
            ..
        }) = event
        else {
            continue;
        };
        let at_line_start = range.start == 0 || text[..range.start].ends_with('\n');
        let line = text[range.start..].lines().next().unwrap_or_default();
        if let Some((title, marker)) = parse_heading(line).filter(|_| at_line_start) {
            headings.push(MarkerHeading {
                start: range.start,
                end: range.start + line.len(),
                title,
                marker,
            });
        }
    }
    headings
}

/// Finds the headings that start a block: the first marker heading of each
/// known block.
fn find_boundaries<'a>(
    body: &'a str,
    ids: &[BlockId],
) -> (Vec<MarkerHeading<'a>>, Vec<DayWarning>) {
    let mut boundaries = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    for heading in marker_headings(body) {
        match ids.iter().find(|id| id.as_str() == heading.marker) {
            Some(id) if seen.insert(id) => boundaries.push(heading),
            Some(id) => warnings.push(DayWarning::DuplicateMarker { id: id.clone() }),
            None => warnings.push(DayWarning::UnknownMarker {
                id: heading.marker.to_owned(),
            }),
        }
    }
    (boundaries, warnings)
}

/// Escapes the headings in `text` that would start one of the blocks `ids`,
/// so that they stay text wherever the text is written.
pub(super) fn escape_block_headings(text: &str, ids: &[BlockId]) -> String {
    let mut escaped = text.to_owned();
    // From the back, so that earlier positions stay valid.
    for heading in marker_headings(text).iter().rev() {
        if ids.iter().any(|id| id.as_str() == heading.marker) {
            escaped.insert(heading.start, '\\');
        }
    }
    escaped
}

/// Splits `## Title {#id}` into title and id.
fn parse_heading(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("## ")?.trim_end();
    let (title, marker) = rest.rsplit_once("{#")?;
    let id = marker.strip_suffix('}')?;
    (title.is_empty() || title.ends_with(' ')).then_some((title.trim(), id))
}

fn strip_date_heading(text: &str, date: NaiveDate) -> &str {
    let heading = format!("# {date}");
    let text = text.trim_start_matches(['\n', '\r']);
    match text.split_once('\n') {
        Some((first, rest)) if first.trim_end() == heading => rest,
        None if text.trim_end() == heading => "",
        _ => text,
    }
}

/// Removes the blank lines around a text. They belong to the layout of the
/// file, not to the text.
pub(super) fn trim_blank_lines(text: &str) -> &str {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        start += line.len();
    }
    text[start..].trim_end()
}
