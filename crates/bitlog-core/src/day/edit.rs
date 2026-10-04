//! Changes to days that keep them valid.

use chrono::NaiveTime;

use super::sections::trim_blank_lines;
use super::{Block, Day};
use crate::{BlockId, EditError, LocationKey, Project, ProjectSlug, VaultConfig};

/// What happens to the text of a removed block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovedText {
    Discard,
    /// Appended to the day note, below a line naming the block.
    MoveToNote,
}

impl Day {
    /// Adds a block and returns its new id.
    pub fn add_block(
        &mut self,
        start: NaiveTime,
        end: NaiveTime,
        project: ProjectSlug,
        title: &str,
        projects: &[Project],
    ) -> Result<BlockId, EditError> {
        check_project(&project, projects)?;
        let title = check_title(title)?;
        let id = BlockId::generate(|id| self.blocks.iter().any(|block| block.id == *id));
        let block = Block {
            id: id.clone(),
            start,
            end,
            project,
            title,
            text: String::new(),
        };
        self.check_times(&block)?;
        self.blocks.push(block);
        self.sort_blocks();
        Ok(id)
    }

    /// Gives the block `id` a new start and end.
    pub fn move_block(
        &mut self,
        id: &BlockId,
        start: NaiveTime,
        end: NaiveTime,
    ) -> Result<(), EditError> {
        let mut block = self.blocks[self.index(id)?].clone();
        block.start = start;
        block.end = end;
        self.check_times(&block)?;
        *self.block_mut(id)? = block;
        self.sort_blocks();
        Ok(())
    }

    pub fn set_block_project(
        &mut self,
        id: &BlockId,
        project: ProjectSlug,
        projects: &[Project],
    ) -> Result<(), EditError> {
        check_project(&project, projects)?;
        self.block_mut(id)?.project = project;
        Ok(())
    }

    /// An empty title removes it.
    pub fn set_block_title(&mut self, id: &BlockId, title: &str) -> Result<(), EditError> {
        let title = check_title(title)?;
        self.block_mut(id)?.title = title;
        Ok(())
    }

    /// Blank lines around `text` are dropped, as they are when reading.
    pub fn set_block_text(&mut self, id: &BlockId, text: &str) -> Result<(), EditError> {
        self.block_mut(id)?.text = trim_blank_lines(text).to_owned();
        Ok(())
    }

    pub fn remove_block(&mut self, id: &BlockId, text: RemovedText) -> Result<(), EditError> {
        let index = self.index(id)?;
        let block = self.blocks.remove(index);
        if text == RemovedText::MoveToNote && !block.text.is_empty() {
            let moved = format!("{}\n\n{}", note_heading(&block), block.text);
            self.note = if self.note.is_empty() {
                moved
            } else {
                format!("{}\n\n{moved}", self.note)
            };
        }
        Ok(())
    }

    /// Sets the location to one of the vault, or removes it.
    pub fn set_location(
        &mut self,
        location: Option<LocationKey>,
        config: &VaultConfig,
    ) -> Result<(), EditError> {
        if let Some(key) = &location
            && !config.locations.contains_key(key)
        {
            return Err(EditError::UnknownLocation(key.clone()));
        }
        self.location = location;
        Ok(())
    }

    /// Checks `block` against all other blocks of the day. Overlaps between
    /// other blocks, as read from a file, do not matter here.
    fn check_times(&self, block: &Block) -> Result<(), EditError> {
        if block.start == block.end {
            return Err(EditError::EmptyBlock);
        }
        match self
            .blocks
            .iter()
            .find(|other| other.id != block.id && other.overlaps(block))
        {
            Some(other) => Err(EditError::Overlap(other.id.clone())),
            None => Ok(()),
        }
    }

    fn sort_blocks(&mut self) {
        self.blocks.sort_by_key(|block| block.span());
    }

    fn index(&self, id: &BlockId) -> Result<usize, EditError> {
        self.blocks
            .iter()
            .position(|block| block.id == *id)
            .ok_or_else(|| EditError::UnknownBlock(id.clone()))
    }

    fn block_mut(&mut self, id: &BlockId) -> Result<&mut Block, EditError> {
        let index = self.index(id)?;
        Ok(&mut self.blocks[index])
    }
}

fn check_project(project: &ProjectSlug, projects: &[Project]) -> Result<(), EditError> {
    if projects.iter().any(|known| known.slug == *project) {
        Ok(())
    } else {
        Err(EditError::UnknownProject(project.clone()))
    }
}

/// The title as it is written into the block heading.
fn check_title(title: &str) -> Result<String, EditError> {
    if title.contains(['\n', '\r']) {
        return Err(EditError::MultilineTitle);
    }
    Ok(title.trim().to_owned())
}

/// `**09:00–10:30 webshop: Title**`, so that moved text keeps its context.
fn note_heading(block: &Block) -> String {
    let mut heading = format!(
        "{}–{} {}",
        block.start.format("%H:%M"),
        block.end.format("%H:%M"),
        block.project
    );
    if !block.title.is_empty() {
        heading = format!("{heading}: {}", block.title);
    }
    format!("**{heading}**")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::NaiveDate;

    use super::*;

    fn sample_path() -> &'static Path {
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/sample-vault"
        ))
    }

    fn projects() -> Vec<Project> {
        Project::load_all(sample_path()).unwrap()
    }

    fn sample_day(date: &str) -> Day {
        let path = sample_path().join(format!("daily/2026/09/{date}.md"));
        Day::read(&path, &std::fs::read_to_string(&path).unwrap())
            .unwrap()
            .0
    }

    fn time(text: &str) -> NaiveTime {
        NaiveTime::parse_from_str(text, "%H:%M").unwrap()
    }

    fn id(value: &str) -> BlockId {
        value.parse().unwrap()
    }

    fn slug(value: &str) -> ProjectSlug {
        value.parse().unwrap()
    }

    fn spans(day: &Day) -> Vec<(String, String)> {
        day.blocks
            .iter()
            .map(|block| {
                (
                    block.start.format("%H:%M").to_string(),
                    block.end.format("%H:%M").to_string(),
                )
            })
            .collect()
    }

    fn new_day() -> Day {
        Day::new(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())
    }

    #[test]
    fn add_blocks_in_order() {
        let mut day = new_day();
        let projects = projects();
        let late = day
            .add_block(
                time("13:00"),
                time("14:00"),
                slug("infra"),
                " Deploy ",
                &projects,
            )
            .unwrap();
        let early = day
            .add_block(time("09:00"), time("10:00"), slug("webshop"), "", &projects)
            .unwrap();
        assert_ne!(late, early);
        assert_eq!(day.blocks[0].id, early);
        assert_eq!(day.blocks[1].title, "Deploy");
        assert_eq!(
            spans(&day),
            [
                ("09:00".into(), "10:00".into()),
                ("13:00".into(), "14:00".into())
            ]
        );

        // Read back with its heading.
        let text = day.to_markdown();
        assert!(text.contains(&format!("## Deploy {{#{late}}}")), "{text}");
        assert_eq!(Day::parse(&text).unwrap().0, day);
    }

    #[test]
    fn blocks_must_not_overlap() {
        let mut day = new_day();
        let projects = projects();
        let first = day
            .add_block(time("09:00"), time("10:00"), slug("infra"), "", &projects)
            .unwrap();
        // Touching is fine.
        day.add_block(time("10:00"), time("10:15"), slug("infra"), "", &projects)
            .unwrap();
        assert_eq!(
            day.add_block(time("08:00"), time("09:15"), slug("infra"), "", &projects),
            Err(EditError::Overlap(first.clone()))
        );
        // Around midnight: 23:00 to 01:00 ends after 23:30.
        day.add_block(time("23:00"), time("01:00"), slug("infra"), "", &projects)
            .unwrap();
        assert!(matches!(
            day.add_block(time("23:30"), time("23:45"), slug("infra"), "", &projects),
            Err(EditError::Overlap(_))
        ));
        // The early morning of the same day is free.
        day.add_block(time("00:15"), time("00:45"), slug("infra"), "", &projects)
            .unwrap();
        assert_eq!(
            day.add_block(time("11:00"), time("11:00"), slug("infra"), "", &projects),
            Err(EditError::EmptyBlock)
        );
        assert_eq!(day.blocks.len(), 4);
    }

    #[test]
    fn move_block() {
        let mut day = sample_day("2026-09-21");
        let first = day.blocks[0].id.clone();
        let second = day.blocks[1].id.clone();
        // Onto the next block.
        assert_eq!(
            day.move_block(&first, time("08:00"), day.blocks[1].end),
            Err(EditError::Overlap(second))
        );
        day.move_block(&first, time("22:00"), time("23:00"))
            .unwrap();
        assert_eq!(day.blocks.last().unwrap().id, first);
        assert_eq!(
            day.move_block(&id("zz99"), time("22:00"), time("23:00")),
            Err(EditError::UnknownBlock(id("zz99")))
        );
    }

    #[test]
    fn existing_overlaps_do_not_block_other_changes() {
        let mut day = new_day();
        let projects = projects();
        day.add_block(time("09:00"), time("10:00"), slug("infra"), "", &projects)
            .unwrap();
        let other = day
            .add_block(time("11:00"), time("12:00"), slug("infra"), "", &projects)
            .unwrap();
        // As if a file had an overlap.
        day.blocks[0].end = time("11:30");
        day.move_block(&other, time("13:00"), time("14:00"))
            .unwrap();
    }

    #[test]
    fn titles_texts_and_projects() {
        let mut day = sample_day("2026-09-21");
        let block = day.blocks[0].id.clone();
        day.set_block_title(&block, "  New title ").unwrap();
        assert_eq!(day.blocks[0].title, "New title");
        assert_eq!(
            day.set_block_title(&block, "two\nlines"),
            Err(EditError::MultilineTitle)
        );
        day.set_block_text(&block, "\n\nSome *text*.\n\n").unwrap();
        assert_eq!(day.blocks[0].text, "Some *text*.");
        day.set_block_project(&block, slug("webshop"), &projects())
            .unwrap();
        assert_eq!(day.blocks[0].project, slug("webshop"));
        assert_eq!(
            day.set_block_project(&block, slug("unknown"), &projects()),
            Err(EditError::UnknownProject(slug("unknown")))
        );
        assert_eq!(Day::parse(&day.to_markdown()).unwrap().0, day);
    }

    #[test]
    fn remove_block() {
        let mut day = new_day();
        let projects = projects();
        let discarded = day
            .add_block(time("08:00"), time("09:00"), slug("infra"), "", &projects)
            .unwrap();
        let moved = day
            .add_block(
                time("09:00"),
                time("10:30"),
                slug("webshop"),
                "Checkout",
                &projects,
            )
            .unwrap();
        let untitled = day
            .add_block(time("11:00"), time("12:00"), slug("infra"), "", &projects)
            .unwrap();
        day.set_block_text(&discarded, "Gone.").unwrap();
        day.set_block_text(&moved, "Kept.").unwrap();
        day.set_block_text(&untitled, "Also kept.").unwrap();

        day.remove_block(&discarded, RemovedText::Discard).unwrap();
        assert_eq!(day.note, "");
        day.remove_block(&moved, RemovedText::MoveToNote).unwrap();
        assert_eq!(day.note, "**09:00–10:30 webshop: Checkout**\n\nKept.");
        day.remove_block(&untitled, RemovedText::MoveToNote)
            .unwrap();
        assert_eq!(
            day.note,
            "**09:00–10:30 webshop: Checkout**\n\nKept.\n\n**11:00–12:00 infra**\n\nAlso kept."
        );
        assert!(day.blocks.is_empty());
        assert_eq!(
            day.remove_block(&moved, RemovedText::Discard),
            Err(EditError::UnknownBlock(moved))
        );
    }

    #[test]
    fn set_location() {
        let config = VaultConfig::load(&sample_path().join("bitlog.toml")).unwrap();
        let mut day = new_day();
        day.set_location(Some("office".parse().unwrap()), &config)
            .unwrap();
        assert_eq!(day.location, Some("office".parse().unwrap()));
        assert_eq!(
            day.set_location(Some("moon".parse().unwrap()), &config),
            Err(EditError::UnknownLocation("moon".parse().unwrap()))
        );
        day.set_location(None, &config).unwrap();
        assert_eq!(day.location, None);
    }
}
