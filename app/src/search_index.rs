//! The index of the open vault, used on worker threads so that the window
//! never waits for it.

use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, TimeDelta};
use gtk::{gio, glib};
use knotbook_core::{NotePath, ProjectSlug, Vault};
use knotbook_index::{Backlink, Index, IndexError, ProjectBlock, SearchHit};

/// What the project page shows of a project.
#[derive(Debug)]
pub struct ProjectData {
    /// The time spent on the project on each day, oldest first.
    pub activity: Vec<(NaiveDate, TimeDelta)>,
    /// The time spent on each project in the month asked for.
    pub month_times: Vec<(ProjectSlug, TimeDelta)>,
    pub longest: Option<ProjectBlock>,
    /// The newest blocks.
    pub blocks: Vec<ProjectBlock>,
}

/// What the reports page shows.
#[derive(Debug)]
pub struct ReportData {
    /// The time spent on each project in the period.
    pub times: Vec<(ProjectSlug, TimeDelta)>,
    /// The time spent on each project on each day of the year.
    pub year: Vec<(NaiveDate, ProjectSlug, TimeDelta)>,
}

/// Clones share the same index.
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    /// `None` until first used. A lock also keeps a search from reading
    /// while an update writes.
    index: Arc<Mutex<Option<Index>>>,
}

impl SearchIndex {
    /// Brings the index up to date with the files of `vault`, opening or
    /// creating it first if needed.
    pub async fn update(&self, vault: &Vault) -> Result<(), IndexError> {
        self.run(vault, refresh).await
    }

    /// Searches the index, see [`Index::search`].
    pub async fn search(
        &self,
        vault: &Vault,
        query: String,
        limit: u32,
    ) -> Result<Vec<SearchHit>, IndexError> {
        self.run(vault, move |index, _| index.search(&query, limit))
            .await
    }

    /// Where the wiki links to `note` lie, after bringing the index up to
    /// date, see [`Index::backlinks`].
    pub async fn backlinks(
        &self,
        vault: &Vault,
        note: NotePath,
    ) -> Result<Vec<Backlink>, IndexError> {
        self.run(vault, move |index, vault| {
            refresh(index, vault)?;
            index.backlinks(&note)
        })
        .await
    }

    /// The time spent on the project `project`, with the time of all
    /// projects in the month from `month.0` to `month.1`, and its newest
    /// `limit` blocks, after bringing the index up to date.
    pub async fn project(
        &self,
        vault: &Vault,
        project: ProjectSlug,
        month: (NaiveDate, NaiveDate),
        limit: u32,
    ) -> Result<ProjectData, IndexError> {
        self.run(vault, move |index, vault| {
            refresh(index, vault)?;
            Ok(ProjectData {
                activity: index.project_activity(&project)?,
                month_times: index.project_time(month.0, month.1)?,
                longest: index.longest_block(&project)?,
                blocks: index.project_blocks(&project, 0, limit)?,
            })
        })
        .await
    }

    /// The time spent on each project from `period.0` to `period.1`, and
    /// per day from `year.0` to `year.1`, after bringing the index up to date.
    pub async fn report(
        &self,
        vault: &Vault,
        period: (NaiveDate, NaiveDate),
        year: (NaiveDate, NaiveDate),
    ) -> Result<ReportData, IndexError> {
        self.run(vault, move |index, vault| {
            refresh(index, vault)?;
            Ok(ReportData {
                times: index.project_time(period.0, period.1)?,
                year: index.project_time_per_day(year.0, year.1)?,
            })
        })
        .await
    }

    /// The blocks from `period.0` to `period.1` as CSV, after bringing the
    /// index up to date, see [`Index::blocks_csv`].
    pub async fn blocks_csv(
        &self,
        vault: &Vault,
        period: (NaiveDate, NaiveDate),
    ) -> Result<String, IndexError> {
        self.run(vault, move |index, vault| {
            refresh(index, vault)?;
            index.blocks_csv(vault, Some(period))
        })
        .await
    }

    /// The remote work days as CSV, after bringing the index up to date,
    /// see [`Index::remote_days_csv`].
    pub async fn remote_days_csv(&self, vault: &Vault) -> Result<String, IndexError> {
        self.run(vault, |index, vault| {
            refresh(index, vault)?;
            index.remote_days_csv()
        })
        .await
    }

    /// More blocks of `project`, see [`Index::project_blocks`].
    pub async fn project_blocks(
        &self,
        vault: &Vault,
        project: ProjectSlug,
        skip: u32,
        limit: u32,
    ) -> Result<Vec<ProjectBlock>, IndexError> {
        self.run(vault, move |index, _| {
            index.project_blocks(&project, skip, limit)
        })
        .await
    }

    async fn run<T: Send + 'static>(
        &self,
        vault: &Vault,
        work: impl FnOnce(&mut Index, &Vault) -> Result<T, IndexError> + Send + 'static,
    ) -> Result<T, IndexError> {
        let index = self.index.clone();
        let vault = vault.clone();
        gio::spawn_blocking(move || {
            let mut index = index.lock().expect("no thread panics holding the index");
            if index.is_none() {
                *index = Some(Index::open(&vault)?);
            }
            work(index.as_mut().expect("the index was just opened"), &vault)
        })
        .await
        .expect("working on the index does not panic")
    }
}

/// Brings `index` up to date with the files of `vault`. Files that cannot be
/// read are left out with a warning; `knotbook doctor` tells more about them.
fn refresh(index: &mut Index, vault: &Vault) -> Result<(), IndexError> {
    for err in index.refresh(vault)? {
        glib::g_warning!("knotbook", "{err}");
    }
    Ok(())
}
