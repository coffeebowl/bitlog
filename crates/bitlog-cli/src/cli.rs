//! The commands and options of `bitlog`, as clap reads them.

use std::path::PathBuf;

use bitlog_core::{BlockId, LocationKey, ProjectSlug, ProjectStatus, TaskId};
use chrono::{NaiveDate, NaiveTime};
use clap::{ArgGroup, Args, Parser, Subcommand};

use crate::edit::parse_span;

/// A daily dev log in plain text files.
#[derive(Parser)]
#[command(name = "bitlog", version = env!("BITLOG_VERSION"))]
pub struct Cli {
    /// The vault folder [default: $BITLOG_VAULT, or else the current folder
    /// or the closest folder above it that holds a bitlog.toml]
    #[arg(long, global = true, value_name = "FOLDER")]
    pub vault: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new vault in the folder given by --vault, or else in the
    /// current folder.
    Init {
        /// The name of the vault [default: the name of its folder]
        #[arg(long)]
        name: Option<String>,
    },
    /// Show today's blocks, working time and location.
    Today,
    /// Show the blocks, working time and location of a day.
    Day {
        /// The day, as in 2026-09-23 [default: today]
        date: Option<NaiveDate>,
    },
    /// Show a summary for a standup: the blocks of the last day with work
    /// and the tasks done on it, then today's blocks and the tasks due.
    Standup {
        /// The day of the standup, as in 2026-09-23 [default: today]
        date: Option<NaiveDate>,
    },
    /// Add, change or remove the blocks of a day.
    Block {
        #[command(subcommand)]
        command: BlockCommand,
        #[command(flatten)]
        date: DateArg,
    },
    /// List, add or change projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// List, add or change the tasks of the global task list.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Check for sync conflict copies, all days for unknown projects,
    /// overlapping blocks, headings in texts and wiki links without their
    /// project in block texts, and all notes for broken wiki links and Git
    /// conflict markers. Exits with 1 if something is left.
    Doctor {
        /// Merge conflict copies into their originals where nothing
        /// contradicts, then escape headings in block texts and day notes,
        /// so that they read as plain text, and name the project of their
        /// block in wiki links without one
        #[arg(long)]
        fix: bool,
    },
    /// Find block titles and texts, day notes, notes and tasks that hold all
    /// the words given. Parts of words count, case and accents do not;
    /// words shorter than three characters are left out.
    Search {
        /// The words to find; put a part in double quotes to find it as
        /// written, as in '"release notes"'
        #[arg(required = true)]
        query: Vec<String>,
        /// Show at most this many results
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Show the latest commits of the current branch in a project's Git
    /// repository on this device.
    Log {
        /// The slug of the project
        project: ProjectSlug,
        /// Show at most this many commits
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Write blocks, remote work days or a week report to the exports
    /// folder of the vault, replacing an earlier export of the same.
    Export {
        #[command(subcommand)]
        command: ExportCommand,
    },
    /// Show the time spent on each project in a period, the current month
    /// by default, and the remote work days of its years.
    #[command(group(ArgGroup::new("period").multiple(false)))]
    Stats {
        /// The current week
        #[arg(long, group = "period")]
        week: bool,
        /// The current month
        #[arg(long, group = "period")]
        month: bool,
        /// The current year
        #[arg(long, group = "period")]
        year: bool,
        /// The first day of the period, as in 2026-09-01
        #[arg(long, group = "period", requires = "to")]
        from: Option<NaiveDate>,
        /// The last day of the period, as in 2026-09-30
        #[arg(long, requires = "from")]
        to: Option<NaiveDate>,
    },
    /// Set the kind or location of a day.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Set {
        /// The kind of day, as in work or vacation
        #[arg(long, group = "change")]
        kind: Option<String>,
        /// A location key from bitlog.toml, as in remote
        #[arg(long, group = "change", conflicts_with = "no_location")]
        location: Option<LocationKey>,
        /// Remove the location
        #[arg(long, group = "change")]
        no_location: bool,
        #[command(flatten)]
        date: DateArg,
    },
}

#[derive(Subcommand)]
pub enum ExportCommand {
    /// All blocks as CSV, or those from --from to --to.
    Blocks {
        /// The first day, as in 2026-09-01
        #[arg(long, requires = "to")]
        from: Option<NaiveDate>,
        /// The last day, as in 2026-09-30
        #[arg(long, requires = "from")]
        to: Option<NaiveDate>,
    },
    /// The remote and hybrid work days of each year as CSV.
    Remote,
    /// A report of a week as Markdown: the time per project, then each day
    /// with its blocks and their texts.
    Week {
        /// A day of the week, as in 2026-09-23 [default: today]
        #[arg(long)]
        date: Option<NaiveDate>,
    },
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// List all projects.
    List,
    /// Add a project.
    #[command(group(ArgGroup::new("change").multiple(true)))]
    Add {
        /// Lowercase letters, digits and hyphens, as in client-portal
        slug: ProjectSlug,
        #[command(flatten)]
        values: ProjectArgs,
    },
    /// Change a project.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Edit {
        /// The slug of the project
        slug: ProjectSlug,
        #[command(flatten)]
        values: ProjectArgs,
        /// active, paused or archived
        #[arg(long, group = "change")]
        status: Option<ProjectStatus>,
        /// Forget the project's repository on this device
        #[arg(long, group = "change", conflicts_with = "repo")]
        no_repo: bool,
    },
    /// Change a project's slug, in all days and notes that refer to it.
    Rename {
        /// The slug of the project
        slug: ProjectSlug,
        /// The new slug
        new_slug: ProjectSlug,
    },
}

#[derive(Subcommand)]
pub enum TaskCommand {
    /// List the open tasks with their ids.
    List {
        /// Also list done and dropped tasks
        #[arg(long)]
        all: bool,
    },
    /// Add a task after the open ones and print its id.
    Add {
        /// The title of the task
        title: String,
        /// The due date, as in 2026-09-30
        #[arg(long)]
        due: Option<NaiveDate>,
    },
    /// Mark a task as done.
    Done {
        /// The id of the task, as shown by `bitlog task list`
        id: TaskId,
    },
    /// Mark a task as dropped, as no longer needed.
    Drop {
        /// The id of the task, as shown by `bitlog task list`
        id: TaskId,
    },
    /// Open a done or dropped task again.
    Reopen {
        /// The id of the task, as shown by `bitlog task list --all`
        id: TaskId,
    },
    /// Change the title or due date of a task.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Edit {
        /// The id of the task, as shown by `bitlog task list`
        id: TaskId,
        /// The new title
        #[arg(long, group = "change")]
        title: Option<String>,
        /// The due date, as in 2026-09-30
        #[arg(long, group = "change", conflicts_with = "no_due")]
        due: Option<NaiveDate>,
        /// Remove the due date
        #[arg(long, group = "change")]
        no_due: bool,
    },
    /// Move a task to another place in the list.
    Move {
        /// The id of the task, as shown by `bitlog task list`
        id: TaskId,
        /// The new place, 1 for the top; open tasks always come before
        /// finished ones
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        position: u32,
    },
    /// Move all done and dropped tasks to the archive of the year they were
    /// finished in, tasks-archive-YYYY.toml.
    Archive,
}

#[derive(Args)]
pub struct ProjectArgs {
    /// The name shown for the project [default for new projects: the slug]
    #[arg(long, group = "change")]
    pub name: Option<String>,
    /// As in 3584e4 or "#3584e4"
    #[arg(long, group = "change")]
    pub color: Option<String>,
    /// Free text; blocks of `break` projects are breaks [default for new
    /// projects: work]
    #[arg(long, group = "change")]
    pub category: Option<String>,
    /// The folder of the project's Git repository on this device
    #[arg(long, group = "change", value_name = "FOLDER")]
    pub repo: Option<PathBuf>,
}

#[derive(Args)]
pub struct DateArg {
    /// The day, as in 2026-09-23 [default: today]
    #[arg(long, global = true)]
    pub date: Option<NaiveDate>,
}

#[derive(Subcommand)]
pub enum BlockCommand {
    /// Add a block and print its id.
    Add {
        /// Start and end, as in 09:00-10:30; an earlier end is on the next day
        #[arg(value_parser = parse_span)]
        span: (NaiveTime, NaiveTime),
        /// The slug of the project
        project: ProjectSlug,
        /// The title of the block
        #[arg(default_value = "")]
        title: String,
    },
    /// Change the time, project or title of a block.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Edit {
        /// The id of the block, as shown by `bitlog day`
        id: BlockId,
        /// New start and end, as in 09:00-10:30
        #[arg(long, group = "change", value_parser = parse_span)]
        time: Option<(NaiveTime, NaiveTime)>,
        /// The slug of the new project
        #[arg(long, group = "change")]
        project: Option<ProjectSlug>,
        /// The new title, empty to remove it
        #[arg(long, group = "change")]
        title: Option<String>,
    },
    /// Edit the text of a block in $VISUAL or $EDITOR.
    Note {
        /// The id of the block, as shown by `bitlog day`
        id: BlockId,
    },
    /// Remove a block.
    Rm {
        /// The id of the block, as shown by `bitlog day`
        id: BlockId,
        /// Append the block's text to the day note
        #[arg(long, conflicts_with = "discard_text")]
        move_text: bool,
        /// Drop the block's text
        #[arg(long)]
        discard_text: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn options_combine() {
        let parse = |args: &str| Cli::try_parse_from(args.split(' ')).map(|_| ());
        parse("bitlog project add docs --name Docs --color ff7800").unwrap();
        parse("bitlog project add docs").unwrap();
        parse("bitlog project edit docs --status paused").unwrap();
        assert!(parse("bitlog project edit docs").is_err());
        parse("bitlog project add docs --repo ../docs").unwrap();
        parse("bitlog project edit docs --no-repo").unwrap();
        assert!(parse("bitlog project edit docs --repo ../docs --no-repo").is_err());
        parse("bitlog project rename docs documentation").unwrap();
        assert!(parse("bitlog project rename docs").is_err());
        assert!(parse("bitlog project rename docs Docs").is_err());
        parse("bitlog task edit t9x2 --title Call --no-due").unwrap();
        assert!(parse("bitlog task edit t9x2").is_err());
        assert!(parse("bitlog task edit t9x2 --due 2026-09-30 --no-due").is_err());
        assert!(parse("bitlog task move t9x2 0").is_err());
        parse("bitlog standup 2026-09-23").unwrap();
        parse("bitlog log webshop --limit 5").unwrap();
        assert!(parse("bitlog log").is_err());
        parse("bitlog search release notes --limit 5").unwrap();
        assert!(parse("bitlog search").is_err());
        parse("bitlog export blocks").unwrap();
        parse("bitlog export blocks --from 2026-09-01 --to 2026-09-30").unwrap();
        assert!(parse("bitlog export blocks --from 2026-09-01").is_err());
        parse("bitlog export week --date 2026-09-23").unwrap();
        parse("bitlog export remote").unwrap();
        parse("bitlog stats").unwrap();
        parse("bitlog stats --week").unwrap();
        parse("bitlog stats --from 2026-09-01 --to 2026-09-30").unwrap();
        assert!(parse("bitlog stats --week --year").is_err());
        assert!(parse("bitlog stats --from 2026-09-01").is_err());
        assert!(parse("bitlog stats --month --from 2026-09-01 --to 2026-09-30").is_err());
    }
}
