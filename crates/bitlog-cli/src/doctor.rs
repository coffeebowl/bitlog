//! `bitlog doctor`: problems in the vault, and fixing what can be fixed.

use std::process;

use anyhow::Result;
use bitlog_core::{Problem, Vault};
use chrono::NaiveDate;

/// Lists the problems in `vault` after fixing what can be fixed, if `fix`,
/// and exits with 1 if some are left.
pub fn doctor(vault: &Vault, fix: bool) -> Result<()> {
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
        println!("\n`bitlog doctor --fix` merges the {mergeable} conflict copies.");
    }
    let escapable = problems.iter().filter(|p| p.can_be_escaped()).count();
    if escapable > 0 {
        println!("\n`bitlog doctor --fix` escapes the {escapable} headings.");
    }
    if problems
        .iter()
        .any(|p| matches!(p, Problem::Conflict { .. }) && !p.can_be_merged())
    {
        println!("\nConflict copies with contradictions have to be merged by hand.");
    }
    process::exit(1);
}
