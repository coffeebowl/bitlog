//! `knotbook`, the command line interface of Knotbook.

use std::env;
use std::path::{self, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use clap::{Parser, Subcommand};
use knotbook_core::Vault;

/// A daily dev log in plain text files.
#[derive(Parser)]
#[command(name = "knotbook", version)]
struct Cli {
    /// The vault folder.
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { name } => init(cli.vault, name),
    }
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
}
