//! Splits the Markdown of a day file into the day note and block sections.

use std::collections::HashSet;

use chrono::NaiveDate;

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

/// Finds the level 2 headings with an ID marker at the start of a line. The
/// Markdown around them does not count: an open code block in one block text
/// must not hide the blocks after it.
fn marker_headings(text: &str) -> Vec<MarkerHeading<'_>> {
    let mut headings = Vec::new();
    let mut start = 0;
    for full_line in text.split_inclusive('\n') {
        let line = full_line.trim_end_matches(['\n', '\r']);
        if let Some((title, marker)) = parse_heading(line) {
            headings.push(MarkerHeading {
                start,
                end: start + line.len(),
                title,
                marker,
            });
        }
        start += full_line.len();
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

/// The escaped headings of the blocks `ids` in `text`, as in
/// `\## Title {#id}`, with the ids. A block heading becomes text like that
/// as the second heading of its block, or when an open code block once hid
/// it, and the text of its block may have come along.
pub(crate) fn escaped_block_headings<'a>(
    text: &'a str,
    ids: &[BlockId],
) -> Vec<(BlockId, &'a str)> {
    text.lines()
        .filter_map(|line| {
            let (_, marker) = parse_heading(line.strip_prefix('\\')?)?;
            let id = ids.iter().find(|id| id.as_str() == marker)?;
            Some((id.clone(), line.trim_end()))
        })
        .collect()
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
pub fn trim_blank_lines(text: &str) -> &str {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        start += line.len();
    }
    text[start..].trim_end()
}
