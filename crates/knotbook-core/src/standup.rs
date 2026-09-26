//! A summary of the last working day and of today for a standup meeting.

use chrono::{Datelike, NaiveDate};

use crate::error::ReadError;
use crate::file::read_optional;
use crate::{Day, ProjectSlug, TaskList, TaskStatus, Vault};

impl Vault {
    /// A standup summary for the day `today` in Markdown, to paste into a
    /// chat. "Yesterday" is the last day before `today` with blocks other
    /// than breaks, with the tasks done on it; "Today" holds the blocks of
    /// `today` and the open tasks due by then. Blocks are listed by project
    /// with their titles, breaks left out.
    pub fn standup(&self, today: NaiveDate) -> Result<String, ReadError> {
        let yesterday = match self.last_work_day(today)? {
            Some(day) => {
                let mut lines = self.project_lines(&day);
                let done = self.tasks_done_on(day.date)?;
                if !done.is_empty() {
                    lines.push(format!("Done: {}", done.join(", ")));
                }
                section("Yesterday", Some(day.date), &lines, "Nothing logged.")
            }
            None => section("Yesterday", None, &[], "Nothing logged."),
        };

        let mut lines = match self.load_day(today)? {
            Some(file) => self.project_lines(&file.day),
            None => Vec::new(),
        };
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
                && file.day.blocks.iter().any(|b| !b.is_break(self.projects()))
            {
                return Ok(Some(file.day));
            }
        }
        Ok(None)
    }

    /// One line per project of the blocks of `day`, in the order they first
    /// appear, with the different titles of its blocks: "Webshop: Code
    /// review, Payment provider switch". Breaks are left out.
    fn project_lines(&self, day: &Day) -> Vec<String> {
        let mut projects: Vec<(&ProjectSlug, Vec<&str>)> = Vec::new();
        for block in day.blocks.iter().filter(|b| !b.is_break(self.projects())) {
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
        projects
            .into_iter()
            .map(|(slug, titles)| {
                let name = self
                    .project(slug)
                    .map_or(slug.as_str(), |project| &project.name);
                if titles.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}: {}", titles.join(", "))
                }
            })
            .collect()
    }

    /// The titles of the tasks done on `date`, also those archived since.
    fn tasks_done_on(&self, date: NaiveDate) -> Result<Vec<String>, ReadError> {
        let path = self.task_archive_path(date.year());
        let archive = match read_optional(&path)? {
            Some(text) => TaskList::read(&path, &text)?,
            None => TaskList::default(),
        };
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
    use crate::file::sample_copy;

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
