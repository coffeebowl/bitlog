//! `bitlog`, the command line interface of BitLog.

use std::collections::BTreeMap;
use std::env;
use std::io::{self, IsTerminal};
use std::path::{self, Path, PathBuf};

use anyhow::{Context, Result, bail};
use bitlog_core::{Period, RemovedText, TaskStatus, Vault, week_start};
use bitlog_index::{Index, export};
use chrono::{Local, NaiveDate};
use clap::Parser;

use crate::cli::{
    BlockCommand, Cli, Command, ExportCommand, ProjectArgs, ProjectCommand, TaskCommand,
};
use crate::doctor::doctor;
use crate::edit::{BlockChanges, DayChanges};
use crate::project::ProjectChanges;
use crate::task::TaskChanges;

mod cli;
mod day;
mod doctor;
mod edit;
mod project;
mod search;
mod stats;
mod table;
mod task;

/// The environment variable naming the vault when --vault is missing.
const VAULT_VARIABLE: &str = "BITLOG_VAULT";

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
            date,
        } => edit::set_day(
            &vault,
            date.date.unwrap_or(today),
            DayChanges {
                kind,
                location: (location.is_some() || no_location).then_some(location),
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
            if let Some(period) = period {
                check_period(period)?;
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
            (export::week_file_name(first), vault.week_report(first)?)
        }
    };
    let path = vault.write_export(&name, &text)?;
    println!("Wrote {}", path.display());
    Ok(())
}

/// Fails for a period that ends before it starts.
fn check_period((first, last): (NaiveDate, NaiveDate)) -> Result<()> {
    if last < first {
        bail!("the period ends on {last}, before it starts on {first}");
    }
    Ok(())
}

fn stats(vault: &Vault, (first, last): (NaiveDate, NaiveDate)) -> Result<()> {
    check_period((first, last))?;
    let index = open_index(vault)?;
    let times = index.project_time(first, last)?;
    let remote = index.remote_days()?;
    print!(
        "{}",
        stats::format_stats(vault, first, last, &times, &remote)
    );
    Ok(())
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
                    "no vault found: {} and the folders above it hold no bitlog.toml; \
                     name the vault with --vault or {VAULT_VARIABLE}",
                    current.display()
                ),
            }
        }
    };
    Ok(Vault::open(&root)?)
}

/// `folder` or the closest folder above it that holds a `bitlog.toml`.
fn find_vault(folder: &Path) -> Option<&Path> {
    folder
        .ancestors()
        .find(|folder| folder.join("bitlog.toml").is_file())
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
        absolute
            .file_name()
            .map_or("BitLog".into(), |name| name.to_string_lossy().into_owned())
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
    use super::*;

    #[test]
    fn find_vault_above() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault");
        let inside = sample.join("daily/2026/09");
        assert_eq!(find_vault(&inside), Some(sample.as_path()));
        assert_eq!(find_vault(&sample), Some(sample.as_path()));
        assert_eq!(find_vault(Path::new("/")), None);
    }
}
