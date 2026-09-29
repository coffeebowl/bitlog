//! A summary of the last working day and of today for a standup meeting.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};

use crate::error::ReadError;
use crate::git_log::own_commits_on;
use crate::{Block, Day, ProjectSlug, TaskList, TaskStatus, Vault};

impl Vault {
    /// A standup summary for the day `today` in Markdown, to paste into a
    /// chat. "Yesterday" is the last day before `today` with blocks other
    /// than breaks, with the tasks done on it; "Today" holds the blocks of
    /// `today` and the open tasks due by then. Blocks are listed by project
    /// with their titles, breaks left out, and with the user's commits of
    /// that day in the project's repository on this device.
    ///
    /// It reads the repositories, so programs with a user interface call
    /// it outside the main thread.
    pub fn standup(&self, today: NaiveDate) -> Result<String, ReadError> {
        let yesterday = match self.last_work_day(today)? {
            Some(day) => {
                let mut lines = self.project_lines(&day.blocks, day.date)?;
                let done = self.tasks_done_on(day.date)?;
                if !done.is_empty() {
                    lines.push(format!("Done: {}", done.join(", ")));
                }
                section("Yesterday", Some(day.date), &lines, "Nothing logged.")
            }
            None => section("Yesterday", None, &[], "Nothing logged."),
        };

        let blocks = match self.load_day(today)? {
            Some(file) => file.day.blocks,
            None => Vec::new(),
        };
        let mut lines = self.project_lines(&blocks, today)?;
        let tasks = self.load_tasks()?;
        let due: Vec<&str> = tasks
            .tasks()
            .iter()
            .filter(|task| task.is_open() && task.due.is_some_and(|due| due <= today))
            .map(|task| task.title.as_str())
            .collect();
        if !due.is_empty() {
            lines.push(format!("Due: {}", due.join(", ")));
        }
        let today = section("Today", Some(today), &lines, "Nothing planned yet.");

        Ok(format!("{yesterday}\n{today}"))
    }

    /// The last day before `today` with blocks other than breaks.
    fn last_work_day(&self, today: NaiveDate) -> Result<Option<Day>, ReadError> {
        for date in self.all_days()?.into_iter().rev() {
            if date >= today {
                continue;
            }
            if let Some(file) = self.load_day(date)?
                && file.day.blocks.iter().any(|b| !self.is_break(&b.project))
            {
                return Ok(Some(file.day));
            }
        }
        Ok(None)
    }

    /// One line per project of `blocks`, in the order they first appear,
    /// with the different titles of its blocks: "Webshop: Code review,
    /// Payment provider switch". Breaks are left out. The commits made on
    /// `date` follow in a nested line, also for projects without blocks.
    fn project_lines(&self, blocks: &[Block], date: NaiveDate) -> Result<Vec<String>, ReadError> {
        let mut commits = self.commits_on(date)?;
        let mut projects: Vec<(&ProjectSlug, Vec<&str>)> = Vec::new();
        for block in blocks.iter().filter(|b| !self.is_break(&b.project)) {
            let found = projects
                .iter()
                .position(|(slug, _)| **slug == block.project);
            let index = found.unwrap_or_else(|| {
                projects.push((&block.project, Vec::new()));
                projects.len() - 1
            });
            let titles = &mut projects[index].1;
            if !block.title.is_empty() && !titles.contains(&block.title.as_str()) {
                titles.push(&block.title);
            }
        }
        let mut lines: Vec<String> = projects
            .into_iter()
            .map(|(slug, titles)| {
                let name = self.project_name(slug);
                let line = if titles.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}: {}", titles.join(", "))
                };
                line + &commits_line(commits.remove(slug))
            })
            .collect();
        lines.extend(commits.into_iter().map(|(slug, commits)| {
            self.project_name(&slug).to_owned() + &commits_line(Some(commits))
        }));
        Ok(lines)
    }

    /// The user's commits made on `date` in the repository of each project
    /// that has one on this device and commits that day. Repositories that
    /// cannot be read, as when moved away, are left out.
    fn commits_on(&self, date: NaiveDate) -> Result<BTreeMap<ProjectSlug, Vec<String>>, ReadError> {
        Ok(self
            .repo_paths()?
            .into_iter()
            .filter_map(|(slug, repo)| {
                let commits = own_commits_on(&repo, date).ok()?;
                (!commits.is_empty()).then_some((slug, commits))
            })
            .collect())
    }

    /// The titles of the tasks done on `date`, also those archived since.
    fn tasks_done_on(&self, date: NaiveDate) -> Result<Vec<String>, ReadError> {
        let archive = TaskList::load(&self.task_archive_path(date.year()))?;
        let tasks = self.load_tasks()?;
        Ok(tasks
            .tasks()
            .iter()
            .chain(archive.tasks())
            .filter(|task| task.status == TaskStatus::Done && task.done == Some(date))
            .map(|task| task.title.clone())
            .collect())
    }
}

/// The nested line under a project with `commits`, if any.
fn commits_line(commits: Option<Vec<String>>) -> String {
    match commits {
        Some(commits) => format!("\n  - Commits: {}", commits.join(", ")),
        None => String::new(),
    }
}

/// A bold heading with the date, if any, and a list of `lines`, or `empty`
/// if there are none.
fn section(heading: &str, date: Option<NaiveDate>, lines: &[String], empty: &str) -> String {
    let mut text = format!("**{heading}**");
    if let Some(date) = date {
        text += &format!(" · {}", date.format("%A, %Y-%m-%d"));
    }
    text += "\n\n";
    if lines.is_empty() {
        text += empty;
        text += "\n";
    }
    for line in lines {
        text += &format!("- {line}\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::file::{TempDir, sample_copy};

    fn sample_vault() -> Vault {
        Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
            .unwrap()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn yesterday_and_today() {
        let standup = sample_vault().standup(date(2026, 9, 23)).unwrap();
        assert_eq!(
            standup,
            "**Yesterday** · Tuesday, 2026-09-22\n\
             \n\
             - Webshop: Code review, Payment provider switch\n\
             - Meetings: Daily\n\
             - Infrastructure: Monitoring alerts\n\
             - Done: Take the keyboard to the office\n\
             \n\
             **Today** · Wednesday, 2026-09-23\n\
             \n\
             - Filler: Mails\n\
             - Meetings: Daily\n\
             - Infrastructure: Prepare release deployment, Release deployment\n\
             - Webshop: Release notes\n"
        );
    }

    /// A repository with the user `ada@example.org` and `commits`, each
    /// by its author at a day and hour of September 2026 in UTC+2, with
    /// its summary and the indexes of its parents. The last one is on
    /// `main`, checked out, and the first given for `feature`.
    fn repository(commits: &[(&str, u32, u32, &str, &[usize])], feature: usize) -> TempDir {
        use git2::{Repository, Signature, Time};
        let dir = TempDir::new();
        let repository = Repository::init(&dir.0).unwrap();
        repository
            .config()
            .unwrap()
            .set_str("user.email", "ada@example.org")
            .unwrap();
        let tree = repository.index().unwrap().write_tree().unwrap();
        let tree = repository.find_tree(tree).unwrap();
        let mut ids = Vec::new();
        for (author, day, hour, summary, parents) in commits {
            let time = date(2026, 9, *day).and_hms_opt(*hour, 0, 0).unwrap();
            let time = Time::new(time.and_utc().timestamp() - 7200, 120);
            let email = format!("{}@example.org", author.to_lowercase());
            let author = Signature::new(author, &email, &time).unwrap();
            let parents: Vec<git2::Commit> = parents
                .iter()
                .map(|i| repository.find_commit(ids[*i]).unwrap())
                .collect();
            let parents: Vec<&git2::Commit> = parents.iter().collect();
            let id = repository
                .commit(None, &author, &author, summary, &tree, &parents)
                .unwrap();
            ids.push(id);
        }
        let main = *ids.last().unwrap();
        repository
            .reference("refs/heads/main", main, true, "")
            .unwrap();
        repository
            .reference("refs/heads/feature", ids[feature], true, "")
            .unwrap();
        repository.set_head("refs/heads/main").unwrap();
        dir
    }

    #[test]
    fn own_commits_by_project() {
        let (_dir, vault) = sample_copy();
        let webshop = repository(
            &[
                ("Ada", 21, 9, "Older", &[]),
                ("Ada", 22, 10, "Fix cart total", &[0]),
                ("Bob", 22, 11, "Bob's work", &[1]),
                ("Ada", 22, 12, "Fix cart total", &[2]),
                ("Ada", 22, 11, "Add checkout tests", &[1]),
                ("Ada", 22, 13, "Merge branch 'feature'", &[3, 4]),
                ("Ada", 23, 9, "Today's fix", &[5]),
            ],
            4,
        );
        let filler = repository(&[("Ada", 22, 16, "Answer mails faster", &[])], 0);
        vault
            .set_repo_path(&"webshop".parse().unwrap(), Some(&webshop.0))
            .unwrap();
        vault
            .set_repo_path(&"filler".parse().unwrap(), Some(&filler.0))
            .unwrap();
        // Moved away since.
        let moved = repository(&[("Ada", 22, 9, "Lost", &[])], 0);
        vault
            .set_repo_path(&"infra".parse().unwrap(), Some(&moved.0))
            .unwrap();
        drop(moved);
        let standup = vault.standup(date(2026, 9, 23)).unwrap();
        assert!(
            standup.contains(
                "- Webshop: Code review, Payment provider switch\n\
                 \x20 - Commits: Fix cart total, Add checkout tests\n\
                 - Meetings: Daily\n\
                 - Infrastructure: Monitoring alerts\n\
                 - Filler\n\
                 \x20 - Commits: Answer mails faster\n\
                 - Done: Take the keyboard to the office\n"
            ),
            "{standup}"
        );
        assert!(
            standup.ends_with("- Webshop: Release notes\n  - Commits: Today's fix\n"),
            "{standup}"
        );
    }

    #[test]
    fn untitled_blocks_show_the_project() {
        let standup = sample_vault().standup(date(2026, 9, 22)).unwrap();
        // The untitled block g7h8 adds nothing to the webshop line.
        assert!(standup.contains("- Webshop: Checkout validation\n"));
        assert!(!standup.contains("Break"));
    }

    #[test]
    fn yesterday_skips_days_without_work() {
        let (_dir, vault) = sample_copy();
        // A day with only a break counts as a day off.
        let file = vault.new_day(date(2026, 9, 25));
        vault
            .update_day(&file, |day| {
                day.add_block(
                    "12:00".parse().unwrap(),
                    "12:30".parse().unwrap(),
                    "pause".parse().unwrap(),
                    "",
                    vault.projects(),
                )
                .map(|_| ())
            })
            .unwrap();
        let standup = vault.standup(date(2026, 9, 28)).unwrap();
        assert!(standup.starts_with("**Yesterday** · Wednesday, 2026-09-23\n"));
        assert!(standup.ends_with("**Today** · Monday, 2026-09-28\n\nNothing planned yet.\n"));
    }

    #[test]
    fn nothing_before_the_first_day() {
        let standup = sample_vault().standup(date(2026, 9, 21)).unwrap();
        assert!(standup.starts_with("**Yesterday**\n\nNothing logged.\n\n**Today**"));
    }

    #[test]
    fn due_tasks_are_for_today() {
        let standup = sample_vault().standup(date(2026, 10, 1)).unwrap();
        assert!(standup.ends_with("\n- Due: Renew the TLS certificate for staging\n"));
    }

    #[test]
    fn archived_tasks_count_as_done() {
        let (_dir, vault) = sample_copy();
        let tasks = vault.load_tasks().unwrap();
        vault.archive_tasks(&tasks, date(2026, 9, 23)).unwrap();
        assert!(fs::exists(vault.task_archive_path(2026)).unwrap());
        let standup = vault.standup(date(2026, 9, 23)).unwrap();
        assert!(standup.contains("- Done: Take the keyboard to the office\n"));
        assert!(!standup.contains("Book a meeting room"));
    }
}
