//! Sums and lookups over the whole vault.

use chrono::{NaiveDate, TimeDelta};
use knotbook_core::{NotePath, ProjectSlug};

use crate::{Found, Index, IndexError};

/// How many work days of a year were spent remote or hybrid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDays {
    pub year: i32,
    /// Work days with the location `remote`.
    pub remote: u32,
    /// Work days with the location `hybrid`, partly remote.
    pub hybrid: u32,
}

impl Index {
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
                let project: String = row.get(0)?;
                let minutes: i64 = row.get(1)?;
                Ok((
                    project.parse().expect("the index holds valid slugs"),
                    TimeDelta::minutes(minutes),
                ))
            })?
            .collect::<Result<_, _>>()?;
        Ok(times)
    }

    /// Where the wiki links to the note `target` lie: notes by project and
    /// name, then days, newest first, each with its day note before its
    /// block texts. A place with more than one link to `target` comes once.
    pub fn backlinks(&self, target: &NotePath) -> Result<Vec<Found>, IndexError> {
        let mut statement = self.connection.prepare_cached(
            "SELECT DISTINCT note_project, note_name, date, block FROM links
             WHERE target_project = ? AND target_name = ?
             ORDER BY note_project IS NULL, note_project, note_name, date DESC, block",
        )?;
        let sources = statement
            .query_map((target.project().as_str(), target.name()), |row| {
                let project: Option<String> = row.get(0)?;
                let name: Option<String> = row.get(1)?;
                let date: Option<NaiveDate> = row.get(2)?;
                let block: Option<String> = row.get(3)?;
                Ok(match (project, name, date, block) {
                    (Some(project), Some(name), _, _) => Found::Note(
                        NotePath::new(project.parse().expect("the index holds valid slugs"), &name)
                            .expect("the index holds valid note names"),
                    ),
                    (_, _, Some(date), Some(block)) => Found::Block {
                        date,
                        id: block.parse().expect("the index holds valid block ids"),
                    },
                    (_, _, Some(date), None) => Found::DayNote(date),
                    _ => unreachable!("a link lies in a note or a day"),
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(sources)
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
