//! `knotbook`, the command line interface of Knotbook.

use std::env;
use std::path::{self, Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, NaiveTime};
use clap::{ArgGroup, Args, Parser, Subcommand};
use knotbook_core::{BlockId, LocationKey, ProjectSlug, RemovedText, Vault};

use crate::edit::{BlockChanges, DayChanges, parse_span};

mod day;
mod edit;

/// The environment variable naming the vault when --vault is missing.
const VAULT_VARIABLE: &str = "KNOTBOOK_VAULT";

/// A daily dev log in plain text files.
#[derive(Parser)]
#[command(name = "knotbook", version)]
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
    /// Add, change or remove the blocks of a day.
    Block {
        #[command(subcommand)]
        command: BlockCommand,
        #[command(flatten)]
        date: DateArg,
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
    match cli.command {
        Command::Init { name } => init(cli.vault, name),
        Command::Today => show_day(&open_vault(cli.vault)?, today),
        Command::Day { date } => show_day(&open_vault(cli.vault)?, date.unwrap_or(today)),
        Command::Block { command, date } => {
            block(&open_vault(cli.vault)?, date.date.unwrap_or(today), command)
        }
        Command::Set {
            kind,
            location,
            no_location,
            work,
            no_work,
            date,
        } => edit::set_day(
            &open_vault(cli.vault)?,
            date.date.unwrap_or(today),
            DayChanges {
                kind,
                location: (location.is_some() || no_location).then_some(location),
                work: (work.is_some() || no_work).then_some(work),
            },
        ),
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
    fn find_vault_above() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault");
        let inside = sample.join("daily/2026/09");
        assert_eq!(find_vault(&inside), Some(sample.as_path()));
        assert_eq!(find_vault(&sample), Some(sample.as_path()));
        assert_eq!(find_vault(Path::new("/")), None);
    }
}
