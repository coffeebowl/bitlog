//! `knotbook`, the command line interface of Knotbook.

use std::collections::BTreeMap;
use std::env;
use std::io::{self, IsTerminal};
use std::path::{self, Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, NaiveTime};
use clap::{ArgGroup, Args, Parser, Subcommand};
use knotbook_core::{
    BlockId, LocationKey, Period, Problem, ProjectSlug, ProjectStatus, RemovedText, TaskId,
    TaskStatus, Vault, week_start,
};
use knotbook_index::{Index, export};

use crate::edit::{BlockChanges, DayChanges, parse_span};
use crate::project::ProjectChanges;
use crate::task::TaskChanges;

mod day;
mod edit;
mod project;
mod search;
mod stats;
mod task;

/// The environment variable naming the vault when --vault is missing.
const VAULT_VARIABLE: &str = "KNOTBOOK_VAULT";

/// A daily dev log in plain text files.
#[derive(Parser)]
#[command(name = "knotbook", version = env!("KNOTBOOK_VERSION"))]
struct Cli {
    /// The vault folder [default: $KNOTBOOK_VAULT, or else the current folder
    /// or the closest folder above it that holds a knotbook.toml]
    #[arg(long, global = true, value_name = "FOLDER")]
    vault: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
    /// overlapping blocks and headings in texts, and all notes for broken
    /// wiki links and Git conflict markers. Exits with 1 if something is
    /// left.
    Doctor {
        /// Merge conflict copies into their originals where nothing
        /// contradicts, then escape headings in block texts and day notes,
        /// so that they read as plain text
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
    /// Set the kind, location or working hours of a day.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Set {
        /// The kind of day, as in work or vacation
        #[arg(long, group = "change")]
        kind: Option<String>,
        /// A location key from knotbook.toml, as in remote
        #[arg(long, group = "change", conflicts_with = "no_location")]
        location: Option<LocationKey>,
        /// Remove the location
        #[arg(long, group = "change")]
        no_location: bool,
        /// The working hours, as in 08:30-16:45
        #[arg(long, group = "change", value_parser = parse_span, conflicts_with = "no_work")]
        work: Option<(NaiveTime, NaiveTime)>,
        /// Remove the working hours
        #[arg(long, group = "change")]
        no_work: bool,
        #[command(flatten)]
        date: DateArg,
    },
}

#[derive(Subcommand)]
enum ExportCommand {
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
enum ProjectCommand {
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
enum TaskCommand {
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
        /// The id of the task, as shown by `knotbook task list`
        id: TaskId,
    },
    /// Mark a task as dropped, as no longer needed.
    Drop {
        /// The id of the task, as shown by `knotbook task list`
        id: TaskId,
    },
    /// Open a done or dropped task again.
    Reopen {
        /// The id of the task, as shown by `knotbook task list --all`
        id: TaskId,
    },
    /// Change the title or due date of a task.
    #[command(group(ArgGroup::new("change").required(true).multiple(true)))]
    Edit {
        /// The id of the task, as shown by `knotbook task list`
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
        /// The id of the task, as shown by `knotbook task list`
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
struct ProjectArgs {
    /// The name shown for the project [default for new projects: the slug]
    #[arg(long, group = "change")]
    name: Option<String>,
    /// As in 3584e4 or "#3584e4"
    #[arg(long, group = "change")]
    color: Option<String>,
    /// Free text; blocks of `break` projects are breaks [default for new
    /// projects: work]
    #[arg(long, group = "change")]
    category: Option<String>,
    /// The folder of the project's Git repository on this device
    #[arg(long, group = "change", value_name = "FOLDER")]
    repo: Option<PathBuf>,
}

#[derive(Args)]
struct DateArg {
    /// The day, as in 2026-09-23 [default: today]
    #[arg(long, global = true)]
    date: Option<NaiveDate>,
}

#[derive(Subcommand)]
enum BlockCommand {
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
        /// The id of the block, as shown by `knotbook day`
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
        /// The id of the block, as shown by `knotbook day`
        id: BlockId,
    },
    /// Remove a block.
    Rm {
        /// The id of the block, as shown by `knotbook day`
        id: BlockId,
        /// Append the block's text to the day note
        #[arg(long, conflicts_with = "discard_text")]
        move_text: bool,
        /// Drop the block's text
        #[arg(long)]
        discard_text: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let today = Local::now().date_naive();
    let command = match cli.command {
        Command::Init { name } => return init(cli.vault, name),
        command => command,
    };
    let vault = open_vault(cli.vault)?;
    match command {
        Command::Init { .. } => unreachable!("a vault is created above"),
        Command::Today => show_day(&vault, today),
        Command::Day { date } => show_day(&vault, date.unwrap_or(today)),
        Command::Standup { date } => {
            print!("{}", vault.standup(date.unwrap_or(today))?);
            Ok(())
        }
        Command::Block { command, date } => block(&vault, date.date.unwrap_or(today), command),
        Command::Doctor { fix } => doctor(&vault, fix),
        Command::Search { query, limit } => search(&vault, &query.join(" "), limit),
        Command::Log { project, limit } => {
            print!("{}", project::log(&vault, &project, limit)?);
            Ok(())
        }
        Command::Export { command } => export(&vault, command, today),
        Command::Stats {
            week,
            month: _,
            year,
            from,
            to,
        } => {
            let period = match (from, to) {
                (Some(from), Some(to)) => (from, to),
                _ => {
                    let period = if week {
                        Period::Week
                    } else if year {
                        Period::Year
                    } else {
                        Period::Month
                    };
                    period.range(today, vault.config().week.first_day)
                }
            };
            stats(&vault, period)
        }
        Command::Project { command } => project(vault, command, today),
        Command::Task { command } => task(&vault, command, today),
        Command::Set {
            kind,
            location,
            no_location,
            work,
            no_work,
            date,
        } => edit::set_day(
            &vault,
            date.date.unwrap_or(today),
            DayChanges {
                kind,
                location: (location.is_some() || no_location).then_some(location),
                work: (work.is_some() || no_work).then_some(work),
            },
        ),
    }
}

/// Opens the index of `vault` and brings it up to date, warning about files
/// that cannot be read.
fn open_index(vault: &Vault) -> Result<Index> {
    let mut index = Index::open(vault)?;
    for err in index.refresh(vault)? {
        eprintln!("warning: {err}");
    }
    Ok(index)
}

fn search(vault: &Vault, query: &str, limit: u32) -> Result<()> {
    let hits = open_index(vault)?.search(query, limit)?;
    // As https://no-color.org asks.
    let color = env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    print!(
        "{}",
        search::format_hits(&hits, color && io::stdout().is_terminal())
    );
    Ok(())
}

fn export(vault: &Vault, command: ExportCommand, today: NaiveDate) -> Result<()> {
    let (name, text) = match command {
        ExportCommand::Blocks { from, to } => {
            let period = from.zip(to);
            if let Some((first, last)) = period
                && last < first
            {
                bail!("the period ends on {last}, before it starts on {first}");
            }
            let text = open_index(vault)?.blocks_csv(vault, period)?;
            (export::blocks_file_name(period), text)
        }
        ExportCommand::Remote => (
            export::REMOTE_DAYS_FILE.to_owned(),
            open_index(vault)?.remote_days_csv()?,
        ),
        ExportCommand::Week { date } => {
            let date = date.unwrap_or(today);
            let first = week_start(date, vault.config().week.first_day);
            (
                export::week_file_name(first),
                export::week_report(vault, first)?,
            )
        }
    };
    let path = vault.write_export(&name, &text)?;
    println!("Wrote {}", path.display());
    Ok(())
}

fn stats(vault: &Vault, (first, last): (NaiveDate, NaiveDate)) -> Result<()> {
    if last < first {
        bail!("the period ends on {last}, before it starts on {first}");
    }
    let index = open_index(vault)?;
    let times = index.project_time(first, last)?;
    let remote = index.remote_days()?;
    print!(
        "{}",
        stats::format_stats(vault, first, last, &times, &remote)
    );
    Ok(())
}

fn doctor(vault: &Vault, fix: bool) -> Result<()> {
    let mut problems = vault.check()?;
    if fix {
        let mut merged = 0;
        for problem in &problems {
            if let Problem::Conflict { copy, .. } = problem
                && problem.can_be_merged()
            {
                vault.merge_conflict(copy, &[])?;
                merged += 1;
            }
        }
        println!("Merged {merged} conflict copies.");
        // Merged days may bring headings of their own.
        problems = vault.check()?;
        let mut dates: Vec<NaiveDate> = problems
            .iter()
            .filter_map(|problem| match problem {
                Problem::Heading { date, .. } => Some(*date),
                _ => None,
            })
            .collect();
        dates.dedup();
        for date in &dates {
            vault.escape_headings(*date)?;
        }
        println!("Escaped the headings of {} days.", dates.len());
        problems = vault.check()?;
    }
    if problems.is_empty() {
        println!("No problems found.");
        return Ok(());
    }
    for problem in &problems {
        println!("{problem}");
    }
    let mergeable = problems.iter().filter(|p| p.can_be_merged()).count();
    if mergeable > 0 {
        println!("\n`knotbook doctor --fix` merges the {mergeable} conflict copies.");
    }
    let escapable = problems.iter().filter(|p| p.can_be_escaped()).count();
    if escapable > 0 {
        println!("\n`knotbook doctor --fix` escapes the {escapable} headings.");
    }
    if problems
        .iter()
        .any(|p| matches!(p, Problem::Conflict { .. }) && !p.can_be_merged())
    {
        println!("\nConflict copies with contradictions have to be merged by hand.");
    }
    process::exit(1);
}

fn project(mut vault: Vault, command: ProjectCommand, today: NaiveDate) -> Result<()> {
    let changes = |values: ProjectArgs, status, no_repo: bool| -> Result<ProjectChanges> {
        let repo = match values.repo {
            Some(repo) => {
                Some(Some(path::absolute(&repo).with_context(|| {
                    format!("cannot find the folder {}", repo.display())
                })?))
            }
            None => no_repo.then_some(None),
        };
        Ok(ProjectChanges {
            name: values.name,
            color: values.color,
            category: values.category,
            status,
            repo,
        })
    };
    match command {
        ProjectCommand::List => {
            let repos = vault.repo_paths().unwrap_or_else(|err| {
                eprintln!("warning: {err}");
                BTreeMap::new()
            });
            print!("{}", project::format_projects(&vault, &repos));
            Ok(())
        }
        ProjectCommand::Add { slug, values } => {
            let changes = changes(values, None, false)?;
            project::add_project(&mut vault, slug, changes, today)
        }
        ProjectCommand::Edit {
            slug,
            values,
            status,
            no_repo,
        } => {
            let changes = changes(values, status, no_repo)?;
            project::edit_project(&mut vault, &slug, changes)
        }
        ProjectCommand::Rename { slug, new_slug } => {
            project::rename_project(&mut vault, &slug, &new_slug)
        }
    }
}

fn task(vault: &Vault, command: TaskCommand, today: NaiveDate) -> Result<()> {
    match command {
        TaskCommand::List { all } => {
            print!("{}", task::format_tasks(&vault.load_tasks()?, all, today));
            Ok(())
        }
        TaskCommand::Add { title, due } => task::add_task(vault, &title, due, today),
        TaskCommand::Done { id } => task::set_status(vault, &id, TaskStatus::Done, today),
        TaskCommand::Drop { id } => task::set_status(vault, &id, TaskStatus::Dropped, today),
        TaskCommand::Reopen { id } => task::set_status(vault, &id, TaskStatus::Open, today),
        TaskCommand::Edit {
            id,
            title,
            due,
            no_due,
        } => task::edit_task(
            vault,
            &id,
            TaskChanges {
                title,
                due: (due.is_some() || no_due).then_some(due),
            },
        ),
        TaskCommand::Move { id, position } => {
            let position = usize::try_from(position).expect("u32 fits into usize");
            task::move_task(vault, &id, position)
        }
        TaskCommand::Archive => task::archive_tasks(vault, today),
    }
}

fn block(vault: &Vault, date: NaiveDate, command: BlockCommand) -> Result<()> {
    match command {
        BlockCommand::Add {
            span,
            project,
            title,
        } => edit::add_block(vault, date, span, project, &title),
        BlockCommand::Edit {
            id,
            time,
            project,
            title,
        } => edit::edit_block(
            vault,
            date,
            &id,
            BlockChanges {
                span: time,
                project,
                title,
            },
        ),
        BlockCommand::Note { id } => edit::edit_block_text(vault, date, &id),
        BlockCommand::Rm {
            id,
            move_text,
            discard_text,
        } => {
            let text = if move_text {
                Some(RemovedText::MoveToNote)
            } else {
                discard_text.then_some(RemovedText::Discard)
            };
            edit::remove_block(vault, date, &id, text)
        }
    }
}

/// Opens the vault given by `--vault`, the environment variable or the
/// current folder, in this order.
fn open_vault(flag: Option<PathBuf>) -> Result<Vault> {
    let from_variable = env::var_os(VAULT_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let root = match flag.or(from_variable) {
        Some(root) => root,
        None => {
            let current = env::current_dir().context("cannot find the current folder")?;
            match find_vault(&current) {
                Some(root) => root.to_owned(),
                None => bail!(
                    "no vault found: {} and the folders above it hold no knotbook.toml; \
                     name the vault with --vault or {VAULT_VARIABLE}",
                    current.display()
                ),
            }
        }
    };
    Ok(Vault::open(&root)?)
}

/// `folder` or the closest folder above it that holds a `knotbook.toml`.
fn find_vault(folder: &Path) -> Option<&Path> {
    folder
        .ancestors()
        .find(|folder| folder.join("knotbook.toml").is_file())
}

fn show_day(vault: &Vault, date: NaiveDate) -> Result<()> {
    let Some(file) = vault.load_day(date)? else {
        println!("{}\nNo entries.", day::heading(date));
        return Ok(());
    };
    for warning in &file.warnings {
        eprintln!("warning: {}: {warning}", vault.day_path(date).display());
    }
    print!("{}", day::format_day(vault, &file.day));
    Ok(())
}

fn init(folder: Option<PathBuf>, name: Option<String>) -> Result<()> {
    let folder = match folder {
        Some(folder) => folder,
        None => env::current_dir().context("cannot find the current folder")?,
    };
    // Absolute, so that `.` has a name as well.
    let absolute = path::absolute(&folder)
        .with_context(|| format!("cannot find the folder {}", folder.display()))?;
    let name = name.unwrap_or_else(|| {
        absolute.file_name().map_or("Knotbook".into(), |name| {
            name.to_string_lossy().into_owned()
        })
    });
    let vault = Vault::create(&absolute, &name, Local::now().date_naive())?;
    println!(
        "Created the vault \u{201c}{}\u{201d} in {}",
        vault.config().name,
        absolute.display()
    );
    Ok(())
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
        parse("knotbook project add docs --name Docs --color ff7800").unwrap();
        parse("knotbook project add docs").unwrap();
        parse("knotbook project edit docs --status paused").unwrap();
        assert!(parse("knotbook project edit docs").is_err());
        parse("knotbook project add docs --repo ../docs").unwrap();
        parse("knotbook project edit docs --no-repo").unwrap();
        assert!(parse("knotbook project edit docs --repo ../docs --no-repo").is_err());
        parse("knotbook project rename docs documentation").unwrap();
        assert!(parse("knotbook project rename docs").is_err());
        assert!(parse("knotbook project rename docs Docs").is_err());
        parse("knotbook task edit t9x2 --title Call --no-due").unwrap();
        assert!(parse("knotbook task edit t9x2").is_err());
        assert!(parse("knotbook task edit t9x2 --due 2026-09-30 --no-due").is_err());
        assert!(parse("knotbook task move t9x2 0").is_err());
        parse("knotbook standup 2026-09-23").unwrap();
        parse("knotbook log webshop --limit 5").unwrap();
        assert!(parse("knotbook log").is_err());
        parse("knotbook search release notes --limit 5").unwrap();
        assert!(parse("knotbook search").is_err());
        parse("knotbook export blocks").unwrap();
        parse("knotbook export blocks --from 2026-09-01 --to 2026-09-30").unwrap();
        assert!(parse("knotbook export blocks --from 2026-09-01").is_err());
        parse("knotbook export week --date 2026-09-23").unwrap();
        parse("knotbook export remote").unwrap();
        parse("knotbook stats").unwrap();
        parse("knotbook stats --week").unwrap();
        parse("knotbook stats --from 2026-09-01 --to 2026-09-30").unwrap();
        assert!(parse("knotbook stats --week --year").is_err());
        assert!(parse("knotbook stats --from 2026-09-01").is_err());
        assert!(parse("knotbook stats --month --from 2026-09-01 --to 2026-09-30").is_err());
    }

    #[test]
    fn find_vault_above() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault");
        let inside = sample.join("daily/2026/09");
        assert_eq!(find_vault(&inside), Some(sample.as_path()));
        assert_eq!(find_vault(&sample), Some(sample.as_path()));
        assert_eq!(find_vault(Path::new("/")), None);
    }
}
