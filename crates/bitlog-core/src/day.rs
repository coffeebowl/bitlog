//! Day files, `daily/YYYY/MM/YYYY-MM-DD.md`.

mod edit;
mod merge;
mod sections;
mod write;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use chrono::{NaiveDate, NaiveTime, TimeDelta, Timelike};
use serde::{Deserialize, Deserializer};

use crate::error::ReadError;
use crate::file::{check_format, parse_text};
use crate::{BlockId, LocationKey, Project, ProjectSlug};

pub use edit::{RemovedText, reordered_spans};
pub(crate) use sections::escaped_block_headings;
pub use sections::trim_blank_lines;

/// Front matter fields of format version 1. Everything else is kept as is.
const KNOWN_FIELDS: [&str; 7] = [
    "format", "date", "kind", "location", "tags", "energy", "blocks",
];

const DEFAULT_KIND: &str = "work";

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub kind: String,
    pub location: Option<LocationKey>,
    pub tags: Vec<String>,
    /// 1 to 5.
    pub energy: Option<u8>,
    /// Sorted by start time.
    pub blocks: Vec<Block>,
    /// The text between the date heading and the first block.
    pub note: String,
    /// Front matter fields this version does not know, in file order.
    pub unknown_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: BlockId,
    pub start: NaiveTime,
    /// Not after `start` if the block ends on the next day.
    pub end: NaiveTime,
    pub project: ProjectSlug,
    /// Empty if the block has no title.
    pub title: String,
    pub text: String,
}

/// Something odd in a day file that does not stop it from being read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DayWarning {
    Overlap {
        first: BlockId,
        second: BlockId,
    },
    /// A heading with an ID marker that is not in the front matter.
    UnknownMarker {
        id: String,
    },
    /// A second heading for the same block.
    DuplicateMarker {
        id: BlockId,
    },
}

impl fmt::Display for DayWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overlap { first, second } => write!(f, "blocks {first} and {second} overlap"),
            Self::UnknownMarker { id } => write!(
                f,
                "the heading marked {{#{id}}} belongs to no block and is read as text"
            ),
            Self::DuplicateMarker { id } => write!(
                f,
                "block {id} has more than one heading, all but the first are read as text"
            ),
        }
    }
}

impl Block {
    /// Start and end in minutes after the start of its day. The end is past
    /// `24 * 60` if the block ends on the next day.
    pub fn span(&self) -> (u32, u32) {
        let start = minute_of_day(self.start);
        (start, start + minutes_until(self.start, self.end))
    }

    pub fn duration(&self) -> TimeDelta {
        TimeDelta::minutes(minutes_until(self.start, self.end).into())
    }

    /// Whether this block and `other` share some time of the day.
    pub fn overlaps(&self, other: &Block) -> bool {
        let ((start, end), (other_start, other_end)) = (self.span(), other.span());
        start < other_end && other_start < end
    }

    /// Start and end as `09:00–10:30`, for text written for people.
    pub(crate) fn times(&self) -> String {
        format!(
            "{}–{}",
            self.start.format("%H:%M"),
            self.end.format("%H:%M")
        )
    }

    /// Whether the block belongs to a `break` project. Blocks of projects
    /// missing from `projects` are work.
    pub(crate) fn is_break(&self, projects: &[Project]) -> bool {
        Project::is_break_in(projects, &self.project)
    }
}

/// Minutes from midnight to `time`, as block spans and the index count them.
pub fn minute_of_day(time: NaiveTime) -> u32 {
    time.hour() * 60 + time.minute()
}

/// The time `minute` minutes after midnight, the other way round than
/// [`minute_of_day`]. The end of the day, minute 1440, is midnight again.
pub fn time_at_minute(minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(minute / 60 % 24, minute % 60, 0)
        .expect("hours and minutes stay in range")
}

/// Minutes from `start` to `end`, where an `end` before `start` lies on the next day.
fn minutes_until(start: NaiveTime, end: NaiveTime) -> u32 {
    let (start, end) = (minute_of_day(start), minute_of_day(end));
    if end < start {
        end + 24 * 60 - start
    } else {
        end - start
    }
}

#[derive(Deserialize)]
struct FrontMatter {
    format: u32,
    #[serde(deserialize_with = "date")]
    date: NaiveDate,
    kind: Option<String>,
    location: Option<LocationKey>,
    #[serde(default)]
    tags: Vec<String>,
    energy: Option<u8>,
    #[serde(default)]
    blocks: Vec<FrontMatterBlock>,
}

#[derive(Deserialize)]
struct FrontMatterBlock {
    id: BlockId,
    #[serde(deserialize_with = "time")]
    start: NaiveTime,
    #[serde(deserialize_with = "time")]
    end: NaiveTime,
    project: ProjectSlug,
}

fn date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveDate, D::Error> {
    let value = String::deserialize(deserializer)?;
    NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|_| {
        serde::de::Error::custom(format!("expected a date \"YYYY-MM-DD\", found {value:?}"))
    })
}

fn time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveTime, D::Error> {
    let value = String::deserialize(deserializer)?;
    NaiveTime::parse_from_str(&value, "%H:%M").map_err(|_| {
        serde::de::Error::custom(format!("expected a time \"HH:MM\", found {value:?}"))
    })
}

impl Day {
    /// Whether this is a working day, not a day off such as a vacation or a
    /// public holiday.
    pub fn is_work(&self) -> bool {
        self.kind == DEFAULT_KIND
    }

    /// A day without any entries.
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            kind: DEFAULT_KIND.to_owned(),
            location: None,
            tags: Vec::new(),
            energy: None,
            blocks: Vec::new(),
            note: String::new(),
            unknown_fields: serde_json::Map::new(),
        }
    }

    /// The block `id` of this day.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.iter().find(|block| block.id == *id)
    }

    /// Reads the content `text` of the day file `path`. Its date has to match
    /// the file name.
    pub fn read(path: &Path, text: &str) -> Result<(Self, Vec<DayWarning>), ReadError> {
        parse_text(path, text, |text| {
            let (day, warnings) = Self::parse(text)?;
            let file_date = path.file_stem().and_then(|stem| stem.to_str());
            if file_date != Some(day.date.to_string().as_str()) {
                return Err(format!("date {} does not match the file name", day.date));
            }
            Ok((day, warnings))
        })
    }

    pub(crate) fn parse(text: &str) -> Result<(Self, Vec<DayWarning>), String> {
        let (yaml, body) = split_front_matter(text)?;
        let front_matter: FrontMatter = serde_saphyr::from_str(yaml).map_err(|e| e.to_string())?;
        // Read a second time without a schema, to keep what this version does not know.
        let all_fields: serde_json::Map<String, serde_json::Value> =
            serde_saphyr::from_str_with_options(
                yaml,
                serde_saphyr::options! { strict_booleans: true },
            )
            .map_err(|e| e.to_string())?;
        let unknown_fields = all_fields
            .into_iter()
            .filter(|(key, _)| !KNOWN_FIELDS.contains(&key.as_str()))
            .collect();

        validate(&front_matter)?;

        let mut blocks: Vec<Block> = front_matter
            .blocks
            .into_iter()
            .map(|block| Block {
                id: block.id,
                start: block.start,
                end: block.end,
                project: block.project,
                title: String::new(),
                text: String::new(),
            })
            .collect();
        blocks.sort_by_key(Block::span);

        let ids: Vec<BlockId> = blocks.iter().map(|block| block.id.clone()).collect();
        let parsed = sections::split(body, front_matter.date, &ids);
        for section in parsed.sections {
            let block = blocks
                .iter_mut()
                .find(|block| block.id.as_str() == section.id)
                .expect("sections only exist for known blocks");
            block.title = section.title.to_owned();
            block.text = section.text.to_owned();
        }

        let mut warnings = overlaps(&blocks);
        warnings.extend(parsed.warnings);

        let day = Self {
            date: front_matter.date,
            kind: front_matter.kind.unwrap_or_else(|| DEFAULT_KIND.to_owned()),
            location: front_matter.location,
            tags: front_matter.tags,
            energy: front_matter.energy,
            blocks,
            note: parsed.note.to_owned(),
            unknown_fields,
        };
        Ok((day, warnings))
    }

    /// The time worked on this day. Blocks of `break` projects are breaks;
    /// blocks of projects missing from `projects` count as work.
    pub fn working_time(&self, projects: &[Project]) -> TimeDelta {
        self.blocks
            .iter()
            .filter(|block| !block.is_break(projects))
            .map(Block::duration)
            .sum()
    }

    /// The time of the blocks of each project, breaks left out.
    pub fn time_per_project(&self, projects: &[Project]) -> BTreeMap<ProjectSlug, TimeDelta> {
        let mut times = BTreeMap::new();
        for block in self.blocks.iter().filter(|block| !block.is_break(projects)) {
            *times.entry(block.project.clone()).or_default() += block.duration();
        }
        times
    }
}

/// `text` without the front matter it starts with, if any, as in notes.
pub fn without_front_matter(text: &str) -> &str {
    split_front_matter(text).map_or(text, |(_, body)| body)
}

/// Splits a file into its front matter, between two lines `---`, and the
/// Markdown below it.
///
/// The front matter slice starts with the newline of the opening `---`, so
/// line numbers in YAML errors match the lines of the file.
fn split_front_matter(text: &str) -> Result<(&str, &str), String> {
    let Some(opening) = text
        .split_inclusive('\n')
        .next()
        .filter(|line| line.trim_end() == "---")
    else {
        return Err("the file has to start with a front matter, a line \"---\"".to_owned());
    };
    let mut end = opening.len();
    for line in text[end..].split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Ok((&text[3..end], &text[end + line.len()..]));
        }
        end += line.len();
    }
    Err("the front matter is not closed by a line \"---\"".to_owned())
}

fn validate(front_matter: &FrontMatter) -> Result<(), String> {
    check_format(front_matter.format)?;
    if let Some(energy) = front_matter.energy
        && !(1..=5).contains(&energy)
    {
        return Err(format!("energy must be between 1 and 5, found {energy}"));
    }
    let mut ids = HashSet::new();
    for block in &front_matter.blocks {
        if !ids.insert(&block.id) {
            return Err(format!("block id {} is used more than once", block.id));
        }
        if block.start == block.end {
            return Err(format!(
                "block {} starts and ends at the same time",
                block.id
            ));
        }
    }
    Ok(())
}

/// Finds blocks that start before an earlier block has ended.
fn overlaps(blocks: &[Block]) -> Vec<DayWarning> {
    let mut warnings = Vec::new();
    let mut latest: Option<&Block> = None;
    for block in blocks {
        if let Some(previous) = latest {
            if block.span().0 < previous.span().1 {
                warnings.push(DayWarning::Overlap {
                    first: previous.id.clone(),
                    second: block.id.clone(),
                });
            }
            if block.span().1 <= previous.span().1 {
                continue;
            }
        }
        latest = Some(block);
    }
    warnings
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn sample_day(date: &str) -> (Day, Vec<DayWarning>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/sample-vault/daily/2026/09")
            .join(format!("{date}.md"));
        Day::read(&path, &fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    fn block<'a>(day: &'a Day, id: &str) -> &'a Block {
        day.block(&id.parse().unwrap()).unwrap()
    }

    fn parse_error(text: &str) -> String {
        Day::parse(text).unwrap_err()
    }

    #[test]
    fn times_at_minutes() {
        assert_eq!(time_at_minute(9 * 60 + 15), time(9, 15));
        assert_eq!(time_at_minute(24 * 60), time(0, 0));
        assert_eq!(minute_of_day(time_at_minute(23 * 60 + 59)), 23 * 60 + 59);
    }

    #[test]
    fn find_block() {
        let (day, _) = sample_day("2026-09-21");
        let first = &day.blocks[0];
        assert_eq!(day.block(&first.id), Some(first));
        assert_eq!(day.block(&"zz99".parse().unwrap()), None);
    }

    #[test]
    fn canonical_day() {
        let (day, warnings) = sample_day("2026-09-21");
        assert!(warnings.is_empty());
        assert_eq!(day.date, NaiveDate::from_ymd_opt(2026, 9, 21).unwrap());
        assert_eq!(day.kind, "work");
        assert_eq!(day.location, Some("remote".parse().unwrap()));
        assert_eq!(day.tags, ["planning"]);
        assert_eq!(day.energy, Some(4));
        assert_eq!(day.note, "Back from vacation, catching up. #planning");
        assert!(day.unknown_fields.is_empty());

        let ids: Vec<&str> = day.blocks.iter().map(|block| block.id.as_str()).collect();
        assert_eq!(ids, ["a1b2", "c3d4", "e5f6", "g7h8", "j9k0", "m1n2"]);

        let mails = block(&day, "a1b2");
        assert_eq!((mails.title.as_str(), mails.text.as_str()), ("Mails", ""));
        assert_eq!(mails.project.as_str(), "filler");
        assert_eq!((mails.start, mails.end), (time(8, 0), time(8, 15)));

        let untitled = block(&day, "g7h8");
        assert_eq!(untitled.title, "");
        assert_eq!(
            untitled.text,
            "Paired with Kim on the rounding bug, see `Price::round`."
        );

        let with_code = block(&day, "c3d4");
        assert!(with_code.text.starts_with("Address validation now runs"));
        assert!(
            with_code
                .text
                .contains("```sh\n## Rebuild the test database\nmake db-reset\n")
        );
        assert!(
            with_code
                .text
                .ends_with("- [ ] Error messages for the **address** step")
        );
    }

    #[test]
    fn reformatted_day() {
        let (day, warnings) = sample_day("2026-09-22");
        assert!(warnings.is_empty());
        assert_eq!(day.kind, "work");
        assert_eq!(day.location, Some("office".parse().unwrap()));
        assert_eq!(day.tags, ["review"]);
        // No date heading, the note starts right after the front matter.
        assert_eq!(day.note, "Office day, lots of reviews.");
        assert_eq!(
            day.unknown_fields.get("mood"),
            Some(&serde_json::Value::from("focused"))
        );

        // Unquoted `10:00` has to stay a time, not become a number.
        let daily = block(&day, "r3s4");
        assert_eq!((daily.start, daily.end), (time(10, 0), time(10, 15)));

        let alerts = block(&day, "t5u6");
        assert!(alerts.text.starts_with("The disk alert on `db-2`"));
        assert!(alerts.text.contains("\n\n## Root cause\n\n"));
        assert!(alerts.text.ends_with("- Added an alert for the backup job"));
        assert_eq!(block(&day, "v7w8").title, "Lunch");
    }

    #[test]
    fn day_with_midnight_block_and_unknown_marker() {
        let (day, warnings) = sample_day("2026-09-23");
        assert_eq!(
            warnings,
            [DayWarning::UnknownMarker {
                id: "zz99".to_owned()
            }]
        );
        assert_eq!(day.location, Some("hybrid".parse().unwrap()));

        let deployment = day.blocks.last().unwrap();
        assert_eq!(deployment.id.as_str(), "ff66");
        assert_eq!(
            (deployment.start, deployment.end),
            (time(22, 30), time(0, 30))
        );
        assert_eq!(deployment.span(), (22 * 60 + 30, 24 * 60 + 30));

        let prepare = block(&day, "cc33");
        assert_eq!(
            prepare.text,
            "Checklist is in [[infra/deployment]].\n\n## Old notes {#zz99}\n\n\
             This section belongs to a block that no longer exists."
        );
        assert_eq!(block(&day, "dd44").title, "Lunch");
    }

    #[test]
    fn minimal_day() {
        let (day, warnings) = Day::parse("---\nformat: 1\ndate: \"2026-01-05\"\n---\n").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(day.kind, "work");
        assert!(day.blocks.is_empty());
        assert_eq!(day.note, "");
    }

    #[test]
    fn working_time_edge_cases() {
        let projects = Project::defaults(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        // `work` was the span of working hours once and is now kept as is.
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\n\
                    work: { start: \"20:00\", end: \"02:00\" }\nblocks:\n  \
                    - { id: \"aaaa\", start: \"21:00\", end: \"23:00\", project: \"pause\" }\n  \
                    - { id: \"bbbb\", start: \"23:00\", end: \"01:00\", project: \"unknown\" }\n\
                    ---\n";
        let day = Day::parse(text).unwrap().0;
        // Blocks of unknown projects count as work, also past midnight.
        assert_eq!(day.working_time(&projects), TimeDelta::hours(2));
        assert!(day.unknown_fields.contains_key("work"));
    }

    #[test]
    fn block_without_heading() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"10:00\", project: \"a\" }\n\
                    ---\n\n# 2026-01-05\n\nJust a note.\n";
        let (day, _) = Day::parse(text).unwrap();
        assert_eq!(day.note, "Just a note.");
        assert_eq!(
            (day.blocks[0].title.as_str(), day.blocks[0].text.as_str()),
            ("", "")
        );
    }

    #[test]
    fn blocks_are_sorted_and_overlaps_reported() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"cccc\", start: \"11:00\", end: \"12:00\", project: \"a\" }\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"11:30\", project: \"a\" }\n  \
                    - { id: \"bbbb\", start: \"10:00\", end: \"10:30\", project: \"a\" }\n\
                    ---\n";
        let (day, warnings) = Day::parse(text).unwrap();
        let ids: Vec<&str> = day.blocks.iter().map(|block| block.id.as_str()).collect();
        assert_eq!(ids, ["aaaa", "bbbb", "cccc"]);
        let overlap = |first: &str, second: &str| DayWarning::Overlap {
            first: first.parse().unwrap(),
            second: second.parse().unwrap(),
        };
        assert_eq!(warnings, [overlap("aaaa", "bbbb"), overlap("aaaa", "cccc")]);
    }

    #[test]
    fn marker_headings_that_are_no_boundary() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"10:00\", project: \"a\" }\n\
                    ---\n\n# 2026-01-05\n\n## Early {#zzzz}\n\n## A {#aaaa}\n\nFirst\n\n\
                    > ## Quoted {#aaaa}\n\n## Again {#aaaa}\n\nSecond\n";
        let (day, warnings) = Day::parse(text).unwrap();
        assert_eq!(day.note, "## Early {#zzzz}");
        assert_eq!(day.blocks[0].title, "A");
        assert_eq!(
            day.blocks[0].text,
            "First\n\n> ## Quoted {#aaaa}\n\n## Again {#aaaa}\n\nSecond"
        );
        assert_eq!(
            warnings,
            [
                DayWarning::UnknownMarker {
                    id: "zzzz".to_owned()
                },
                DayWarning::DuplicateMarker {
                    id: "aaaa".parse().unwrap()
                },
            ]
        );
    }

    #[test]
    fn open_code_block_hides_no_block() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"10:00\", project: \"a\" }\n  \
                    - { id: \"bbbb\", start: \"10:00\", end: \"11:00\", project: \"a\" }\n\
                    ---\n\n# 2026-01-05\n\n## A {#aaaa}\n\nFirst\n\n```\n\n<!-- open\n\n\
                    ## B {#bbbb}\n\nSecond\n";
        let (day, warnings) = Day::parse(text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(day.blocks[0].text, "First\n\n```\n\n<!-- open");
        assert_eq!(
            (day.blocks[1].title.as_str(), day.blocks[1].text.as_str()),
            ("B", "Second")
        );
        assert_eq!(Day::parse(&day.to_markdown()).unwrap().0, day);
    }

    #[test]
    fn front_matter_of_notes() {
        assert_eq!(
            without_front_matter("---\ntags: [a]\n---\n\nText\n"),
            "\nText\n"
        );
        assert_eq!(
            without_front_matter("--- \r\ntags: [a]\r\n---\r\nText"),
            "Text"
        );
        for text in ["Text\n---\n", "---\nNot closed\n", "-----\nRule\n---\n", ""] {
            assert_eq!(without_front_matter(text), text);
        }
    }

    #[test]
    fn invalid_front_matter() {
        assert!(parse_error("format: 1\n").contains("has to start with a front matter"));
        assert!(parse_error("---\nformat: 1\n").contains("not closed"));
        assert!(parse_error("---\nformat: 2\ndate: \"2026-01-05\"\n---\n").contains("version 2"));
        assert!(
            parse_error("---\nformat: 1\ndate: \"2026-01-05\"\nenergy: 6\n---\n")
                .contains("energy")
        );

        let bad_time = parse_error(
            "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
             - { id: \"aaaa\", start: \"9 Uhr\", end: \"10:00\", project: \"a\" }\n---\n",
        );
        assert!(bad_time.contains("line 5"), "{bad_time}");
        assert!(bad_time.contains("expected a time \"HH:MM\""), "{bad_time}");

        let block = |id: &str, start: &str, end: &str| {
            format!("  - {{ id: \"{id}\", start: \"{start}\", end: \"{end}\", project: \"a\" }}\n")
        };
        let day =
            |blocks: &str| format!("---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n{blocks}---\n");
        let duplicate = day(&(block("aaaa", "09:00", "10:00") + &block("aaaa", "10:00", "11:00")));
        assert!(parse_error(&duplicate).contains("more than once"));
        assert!(parse_error(&day(&block("aaaa", "09:00", "09:00"))).contains("same time"));
        assert!(parse_error(&day(&block("AAAA", "09:00", "10:00"))).contains("invalid block id"));
    }

    #[test]
    fn date_has_to_match_file_name() {
        let path = Path::new("/vault/2026-01-06.md");
        let err = Day::read(path, "---\nformat: 1\ndate: \"2026-01-05\"\n---\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("2026-01-06.md"), "{err}");
        assert!(err.contains("does not match the file name"), "{err}");
    }
}
