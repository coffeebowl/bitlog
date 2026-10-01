//! Full-text search over the texts of a vault.

use std::ops::Range;

use bitlog_core::{BlockId, NotePath, TaskId};
use chrono::NaiveDate;
use rusqlite::Row;

use crate::{Index, IndexError, note_path, parsed};

/// Marks the start and end of a match in a snippet from SQLite.
const MATCH_START: char = '\u{2}';
const MATCH_END: char = '\u{3}';

/// About as many characters as a snippet holds.
const SNIPPET_LENGTH: u32 = 64;

/// Titles and note names weigh this much more than texts.
const TITLE_WEIGHT: f64 = 5.0;

/// Where a search found something, or where a wiki link lies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The title or text of a block.
    Block {
        date: NaiveDate,
        id: BlockId,
    },
    DayNote(NaiveDate),
    /// The name or text of a note.
    Note(NotePath),
    Task(TaskId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub found: Found,
    /// A part of the text around the matches, `…` where it is cut.
    pub snippet: String,
    /// Byte ranges of the matches in `snippet`.
    pub matches: Vec<Range<usize>>,
}

impl Index {
    /// Finds the blocks, day notes, notes and tasks that hold all words of
    /// `query`, best matches first, at most `limit`. A part in double quotes
    /// is found as it is written, spaces included. Case and accents do not
    /// matter, and parts of words count.
    ///
    /// Words shorter than three characters are left out, the index cannot
    /// find them. A query with no other words finds nothing.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, IndexError> {
        let Some(query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let marks = format!(
            "char({}), char({}), '…', {SNIPPET_LENGTH}",
            MATCH_START as u32, MATCH_END as u32
        );
        let sql = format!(
            "SELECT 0 AS kind, b.date AS date, b.id AS key, NULL AS project,
                    snippet(blocks_search, -1, {marks}) AS snippet,
                    bm25(blocks_search, {TITLE_WEIGHT}, 1.0) AS rank
             FROM blocks_search JOIN blocks b ON b.rowid = blocks_search.rowid
             WHERE blocks_search MATCH ?1
             UNION ALL
             SELECT 1, d.date, NULL, NULL, snippet(days_search, 0, {marks}), bm25(days_search)
             FROM days_search JOIN days d ON d.rowid = days_search.rowid
             WHERE days_search MATCH ?1
             UNION ALL
             SELECT 2, NULL, n.name, n.project, snippet(notes_search, -1, {marks}),
                    bm25(notes_search, {TITLE_WEIGHT}, 1.0)
             FROM notes_search JOIN notes n ON n.rowid = notes_search.rowid
             WHERE notes_search MATCH ?1
             UNION ALL
             SELECT 3, NULL, t.id, NULL, snippet(tasks_search, 0, {marks}), bm25(tasks_search)
             FROM tasks_search JOIN tasks t ON t.rowid = tasks_search.rowid
             WHERE tasks_search MATCH ?1
             ORDER BY rank, date DESC
             LIMIT ?2"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let hits = statement
            .query_map(rusqlite::params![query, limit], |row| {
                let snippet: String = row.get("snippet")?;
                let (snippet, matches) = unmark(&snippet);
                Ok(SearchHit {
                    found: found(row)?,
                    snippet,
                    matches,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }
}

/// What a row of the search query points to, by its columns `kind`,
/// `date`, `key` and `project`.
fn found(row: &Row) -> rusqlite::Result<Found> {
    Ok(match row.get::<_, u8>(0)? {
        0 => Found::Block {
            date: row.get(1)?,
            id: parsed(row, 2)?,
        },
        1 => Found::DayNote(row.get(1)?),
        2 => Found::Note(note_path(row, 3, 2)?),
        3 => Found::Task(parsed(row, 2)?),
        _ => unreachable!("the search query has four kinds"),
    })
}

/// The FTS5 query for what a user typed: each word, or part in double
/// quotes, as a phrase of its own, so that FTS5 syntax has no effect and all
/// of them have to appear. `None` if nothing is left to search for.
fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    // Every second part lies between quotes.
    for (index, part) in input.split('"').enumerate() {
        if index % 2 == 1 {
            terms.push(part.trim());
        } else {
            terms.extend(part.split_whitespace());
        }
    }
    let phrases: Vec<String> = terms
        .into_iter()
        .filter(|term| term.chars().count() >= 3)
        .map(|term| format!("\"{term}\""))
        .collect();
    (!phrases.is_empty()).then(|| phrases.join(" "))
}

/// Takes the match marks out of a snippet and returns where they were.
fn unmark(marked: &str) -> (String, Vec<Range<usize>>) {
    let mut text = String::with_capacity(marked.len());
    let mut matches = Vec::new();
    let mut start = 0;
    for c in marked.chars() {
        match c {
            MATCH_START => start = text.len(),
            MATCH_END => matches.push(start..text.len()),
            c => text.push(c),
        }
    }
    (text, matches)
}
