//! Search results as the terminal shows them.

use knotbook_index::{Found, SearchHit};

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// One line per hit: where it lies, then the snippet on one line, with the
/// matches in bold if `bold`.
pub fn format_hits(hits: &[SearchHit], bold: bool) -> String {
    if hits.is_empty() {
        return "Nothing found.\n".to_owned();
    }
    let places: Vec<String> = hits.iter().map(|hit| place(&hit.found)).collect();
    let width = places
        .iter()
        .map(|place| place.chars().count())
        .max()
        .unwrap_or(0);
    hits.iter()
        .zip(&places)
        .map(|(hit, place)| format!("{place:<width$}  {}\n", one_line(hit, bold)))
        .collect()
}

/// Where a hit lies, notes as a wiki link would name them.
fn place(found: &Found) -> String {
    match found {
        Found::Block { date, id } => format!("{date} {id}"),
        Found::DayNote(date) => format!("{date} day note"),
        Found::Note(note) => format!("{}/{}", note.project(), note.name()),
        Found::Task(id) => format!("task {id}"),
    }
}

/// The snippet of `hit` with its line breaks and runs of spaces as single
/// spaces.
fn one_line(hit: &SearchHit, bold: bool) -> String {
    let mut line = String::new();
    let mut in_match = false;
    for (index, c) in hit.snippet.char_indices() {
        let starts_match = hit.matches.iter().any(|range| range.contains(&index));
        if bold && starts_match != in_match {
            line.push_str(if starts_match { BOLD } else { RESET });
        }
        in_match = starts_match;
        if !c.is_whitespace() {
            line.push(c);
        } else if !line.ends_with(' ') {
            line.push(' ');
        }
    }
    if bold && in_match {
        line.push_str(RESET);
    }
    line.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use knotbook_core::NotePath;

    use super::*;

    fn hit(found: Found, snippet: &str, word: &str) -> SearchHit {
        let start = snippet.find(word).unwrap();
        SearchHit {
            found,
            snippet: snippet.to_owned(),
            matches: std::iter::once(start..start + word.len()).collect(),
        }
    }

    #[test]
    fn hits_on_one_line_each() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let note = NotePath::new("infra".parse().unwrap(), "deployment").unwrap();
        let block = Found::Block {
            date,
            id: "ff66".parse().unwrap(),
        };
        let hits = [
            hit(block, "Release deployment", "deploy"),
            hit(Found::DayNote(date), "Deployed  late.", "Deploy"),
            hit(Found::Note(note), "# Deployment\n\nReleases…", "Deploy"),
            hit(Found::Task("h4c8".parse().unwrap()), "Deploy TLS", "Deploy"),
        ];
        let expected = "\
2026-09-23 ff66      Release deployment
2026-09-23 day note  Deployed late.
infra/deployment     # Deployment Releases…
task h4c8            Deploy TLS
";
        assert_eq!(format_hits(&hits, false), expected);
        assert_eq!(one_line(&hits[0], true), "Release \x1b[1mdeploy\x1b[0mment");
        assert_eq!(format_hits(&[], false), "Nothing found.\n");
    }
}
