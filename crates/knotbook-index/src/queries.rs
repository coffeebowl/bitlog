//! Sums and lookups over the whole vault.

use chrono::{NaiveDate, TimeDelta};
use knotbook_core::{BlockId, NotePath, ProjectSlug};

use rusqlite::OptionalExtension;

use crate::{Found, Index, IndexError, note_path, parsed};

/// How many work days of a year were spent remote or hybrid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDays {
    pub year: i32,
    /// Work days with the location `remote`.
    pub remote: u32,
    /// Work days with the location `hybrid`, partly remote.
    pub hybrid: u32,
}

/// A block of one project, as the project's timeline shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBlock {
    pub date: NaiveDate,
    pub id: BlockId,
    /// Minutes after the start of the day. The end is past `24 * 60` if the
    /// block ends on the next day.
    pub start_minute: u32,
    pub end_minute: u32,
    pub title: String,
    pub text: String,
}

impl ProjectBlock {
    pub fn duration(&self) -> TimeDelta {
        TimeDelta::minutes((self.end_minute - self.start_minute).into())
    }
}

/// A place with a wiki link to a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub found: Found,
    /// The title of the block the link lies in, if it has one.
    pub title: Option<String>,
}

const PROJECT_BLOCK_COLUMNS: &str = "date, id, start_minute, end_minute, title, text";

fn project_block(row: &rusqlite::Row) -> rusqlite::Result<ProjectBlock> {
    Ok(ProjectBlock {
        date: row.get(0)?,
        id: parsed(row, 1)?,
        start_minute: row.get(2)?,
        end_minute: row.get(3)?,
        title: row.get(4)?,
        text: row.get(5)?,
    })
}

impl Index {
    /// The time spent on each project on each day from `first` to `last`,
    /// both included, oldest first. A block counts on the day it starts.
    pub fn project_time_per_day(
        &self,
        first: NaiveDate,
        last: NaiveDate,
    ) -> Result<Vec<(NaiveDate, ProjectSlug, TimeDelta)>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT date, project, sum(end_minute - start_minute) FROM blocks
             WHERE date BETWEEN ? AND ? GROUP BY date, project ORDER BY date, project",
        )?;
        let times = statement
            .query_map((first, last), |row| {
                Ok((
                    row.get(0)?,
                    parsed(row, 1)?,
                    TimeDelta::minutes(row.get(2)?),
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(times)
    }

    /// The time spent on `project` on each day it was worked on, oldest
    /// first. A block counts on the day it starts.
    pub fn project_activity(
        &self,
        project: &ProjectSlug,
    ) -> Result<Vec<(NaiveDate, TimeDelta)>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT date, sum(end_minute - start_minute) FROM blocks
             WHERE project = ? GROUP BY date ORDER BY date",
        )?;
        let days = statement
            .query_map([project.as_str()], |row| {
                Ok((row.get(0)?, TimeDelta::minutes(row.get(1)?)))
            })?
            .collect::<Result<_, _>>()?;
        Ok(days)
    }

    /// All blocks with their projects from `first` to `last`, both included
    /// and each open if `None`, oldest first.
    pub fn blocks_between(
        &self,
        first: Option<NaiveDate>,
        last: Option<NaiveDate>,
    ) -> Result<Vec<(ProjectSlug, ProjectBlock)>, IndexError> {
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT {PROJECT_BLOCK_COLUMNS}, project FROM blocks
             WHERE (?1 IS NULL OR date >= ?1) AND (?2 IS NULL OR date <= ?2)
             ORDER BY date, start_minute"
        ))?;
        let blocks = statement
            .query_map((first, last), |row| {
                Ok((parsed(row, 6)?, project_block(row)?))
            })?
            .collect::<Result<_, _>>()?;
        Ok(blocks)
    }

    /// The blocks of `project`, newest first, leaving out the first `skip`
    /// and returning at most `limit`.
    pub fn project_blocks(
        &self,
        project: &ProjectSlug,
        skip: u32,
        limit: u32,
    ) -> Result<Vec<ProjectBlock>, IndexError> {
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT {PROJECT_BLOCK_COLUMNS} FROM blocks WHERE project = ?
             ORDER BY date DESC, start_minute DESC LIMIT ? OFFSET ?"
        ))?;
        let blocks = statement
            .query_map((project.as_str(), limit, skip), project_block)?
            .collect::<Result<_, _>>()?;
        Ok(blocks)
    }

    /// The longest block of `project`, the newest of them if several are as
    /// long.
    pub fn longest_block(&self, project: &ProjectSlug) -> Result<Option<ProjectBlock>, IndexError> {
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT {PROJECT_BLOCK_COLUMNS} FROM blocks WHERE project = ?
             ORDER BY end_minute - start_minute DESC, date DESC LIMIT 1"
        ))?;
        Ok(statement
            .query_row([project.as_str()], project_block)
            .optional()?)
    }

    /// The time spent on each project from `first` to `last`, both
    /// included, most first. Breaks and projects that do not exist count as
    /// well. A block counts on the day it starts.
    pub fn project_time(
        &self,
        first: NaiveDate,
        last: NaiveDate,
    ) -> Result<Vec<(ProjectSlug, TimeDelta)>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT project, sum(end_minute - start_minute) AS minutes FROM blocks
             WHERE date BETWEEN ? AND ?
             GROUP BY project ORDER BY minutes DESC, project",
        )?;
        let times = statement
            .query_map((first, last), |row| {
                Ok((parsed(row, 0)?, TimeDelta::minutes(row.get(1)?)))
            })?
            .collect::<Result<_, _>>()?;
        Ok(times)
    }

    /// The last day each project was worked on, by project. Projects that
    /// do not exist count as well. A block counts on the day it starts.
    pub fn last_days(&self) -> Result<Vec<(ProjectSlug, NaiveDate)>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT project, max(date) FROM blocks GROUP BY project ORDER BY project",
        )?;
        let days = statement
            .query_map([], |row| Ok((parsed(row, 0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(days)
    }

    /// Where the wiki links to the note `target` lie: notes by project and
    /// name, then days, newest first, each with its day note before its
    /// block texts. A place with more than one link to `target` comes once.
    pub fn backlinks(&self, target: &NotePath) -> Result<Vec<Backlink>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT DISTINCT l.note_project, l.note_name, l.date, l.block, b.title FROM links l
             LEFT JOIN blocks b ON b.date = l.date AND b.id = l.block
             WHERE l.target_project = ? AND l.target_name = ?
             ORDER BY l.note_project IS NULL, l.note_project, l.note_name, l.date DESC, l.block",
        )?;
        let backlinks = statement
            .query_map((target.project().as_str(), target.name()), |row| {
                let project: Option<String> = row.get(0)?;
                let date: Option<NaiveDate> = row.get(2)?;
                let block: Option<String> = row.get(3)?;
                let found = match (project, date, block) {
                    (Some(_), _, _) => Found::Note(note_path(row, 0, 1)?),
                    (None, Some(date), Some(_)) => Found::Block {
                        date,
                        id: parsed(row, 3)?,
                    },
                    (None, Some(date), None) => Found::DayNote(date),
                    // A check of the table keeps links out that lie nowhere.
                    (None, None, _) => unreachable!("a link lies in a note or a day"),
                };
                let title: Option<String> = row.get(4)?;
                Ok(Backlink {
                    found,
                    title: title.filter(|title| !title.is_empty()),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(backlinks)
    }

    /// The remote and hybrid work days of each year that has any, oldest
    /// first. Days of other kinds, such as vacation, do not count, wherever
    /// they were spent.
    pub fn remote_days(&self) -> Result<Vec<RemoteDays>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT CAST(substr(date, 1, 4) AS INTEGER) AS year,
                    count(*) FILTER (WHERE location = 'remote'),
                    count(*) FILTER (WHERE location = 'hybrid')
             FROM days
             WHERE kind = 'work' AND location IN ('remote', 'hybrid')
             GROUP BY year ORDER BY year",
        )?;
        let days = statement
            .query_map([], |row| {
                Ok(RemoteDays {
                    year: row.get(0)?,
                    remote: row.get(1)?,
                    hybrid: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(days)
    }
}
