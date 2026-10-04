//! Commands that change a day.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process;

use anyhow::{Context, Result, bail};
use bitlog_core::{BlockId, Day, DayFile, EditError, LocationKey, ProjectSlug, RemovedText, Vault};
use chrono::{NaiveDate, NaiveTime};

/// Applies `change` to the day `date`, creating its file if needed.
pub fn update_day(
    vault: &Vault,
    date: NaiveDate,
    change: impl FnOnce(&mut Day) -> Result<(), EditError>,
) -> Result<DayFile> {
    let file = match vault.load_day(date)? {
        Some(file) => file,
        None => vault.new_day(date),
    };
    Ok(vault.update_day(&file, change)?)
}

/// Parses a span like `09:00-10:30`. An end before the start lies on the
/// next day.
pub fn parse_span(text: &str) -> Result<(NaiveTime, NaiveTime), String> {
    let (start, end) = text
        .split_once(['-', '–'])
        .ok_or_else(|| format!("expected a span like 09:00-10:30, found {text:?}"))?;
    let time = |part: &str| {
        NaiveTime::parse_from_str(part.trim(), "%H:%M")
            .map_err(|_| format!("expected a time like 09:00, found {part:?}"))
    };
    Ok((time(start)?, time(end)?))
}

pub fn add_block(
    vault: &Vault,
    date: NaiveDate,
    (start, end): (NaiveTime, NaiveTime),
    project: ProjectSlug,
    title: &str,
) -> Result<()> {
    let mut id = None;
    update_day(vault, date, |day| {
        id = Some(day.add_block(start, end, project, title, vault.projects())?);
        Ok(())
    })?;
    println!("Added block {}", id.expect("the block was added"));
    Ok(())
}

pub struct BlockChanges {
    pub span: Option<(NaiveTime, NaiveTime)>,
    pub project: Option<ProjectSlug>,
    pub title: Option<String>,
}

pub fn edit_block(
    vault: &Vault,
    date: NaiveDate,
    id: &BlockId,
    changes: BlockChanges,
) -> Result<()> {
    update_day(vault, date, |day| {
        if let Some((start, end)) = changes.span {
            day.move_block(id, start, end)?;
        }
        if let Some(project) = changes.project {
            day.set_block_project(id, project, vault.projects())?;
        }
        if let Some(title) = &changes.title {
            day.set_block_title(id, title)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// Removes a block. A block with text needs `text` to say what happens to it.
pub fn remove_block(
    vault: &Vault,
    date: NaiveDate,
    id: &BlockId,
    text: Option<RemovedText>,
) -> Result<()> {
    let has_text = vault.load_day(date)?.is_some_and(|file| {
        file.day
            .block(id)
            .is_some_and(|block| !block.text.is_empty())
    });
    let text = match text {
        Some(text) => text,
        None if has_text => bail!(
            "block {id} has text; pass --move-text to keep it in the day note, or --discard-text"
        ),
        None => RemovedText::Discard,
    };
    update_day(vault, date, |day| day.remove_block(id, text))?;
    Ok(())
}

/// Opens the text of a block in the user's editor and saves it afterwards.
pub fn edit_block_text(vault: &Vault, date: NaiveDate, id: &BlockId) -> Result<()> {
    let file = vault
        .load_day(date)?
        .ok_or(EditError::UnknownBlock(id.clone()))?;
    let block = file
        .day
        .block(id)
        .ok_or(EditError::UnknownBlock(id.clone()))?;
    // A new file under a name nobody knows in advance, readable only by
    // this user, so that nobody else can read the text or slip in a file.
    let name = format!("bitlog-{date}-{id}-{:08x}.md", fastrand::u32(..));
    let path = env::temp_dir().join(name);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut copy| writeln!(copy, "{}", block.text))
        .with_context(|| format!("cannot write {}", path.display()))?;
    let edited = run_editor(&path).and_then(|()| {
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))
    });
    // The copy is only needed while editing.
    let _ = fs::remove_file(&path);
    let text = edited?;
    vault.update_day(&file, |day| day.set_block_text(id, &text))?;
    Ok(())
}

/// Runs `$VISUAL` or `$EDITOR` like Git does, through the shell, so that
/// values like `code --wait` work.
fn run_editor(path: &std::path::Path) -> Result<()> {
    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| "vi".to_owned());
    let status = process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("cannot start the editor {editor}"))?;
    if !status.success() {
        bail!("the editor {editor} failed, the text was not saved");
    }
    Ok(())
}

pub struct DayChanges {
    pub kind: Option<String>,
    /// `Some(None)` removes the location.
    pub location: Option<Option<LocationKey>>,
}

pub fn set_day(vault: &Vault, date: NaiveDate, changes: DayChanges) -> Result<()> {
    update_day(vault, date, |day| {
        if let Some(kind) = changes.kind {
            day.kind = kind;
        }
        if let Some(location) = changes.location {
            day.set_location(location, vault.config())?;
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn spans() {
        assert_eq!(parse_span("09:00-10:30"), Ok((time(9, 0), time(10, 30))));
        assert_eq!(parse_span("9:00 – 10:30"), Ok((time(9, 0), time(10, 30))));
        assert_eq!(parse_span("23:00-01:00"), Ok((time(23, 0), time(1, 0))));
        for invalid in ["09:00", "09:00-", "9-10", "25:00-26:00"] {
            assert!(parse_span(invalid).is_err(), "{invalid}");
        }
    }
}
