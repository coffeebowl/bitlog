//! Day files, `daily/YYYY/MM/YYYY-MM-DD.md`.

mod sections;
mod write;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use chrono::{NaiveDate, NaiveTime, TimeDelta, Timelike};
use serde::{Deserialize, Deserializer};

use crate::error::{ReadError, read_file};
use crate::{BlockId, LocationKey, Project, ProjectSlug};

/// The only format version this code knows.
const FORMAT: u32 = 1;

/// Front matter fields of format version 1. Everything else is kept as is.
const KNOWN_FIELDS: [&str; 8] = [
    "format", "date", "kind", "location", "tags", "energy", "work", "blocks",
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
    pub work_start: Option<NaiveTime>,
    pub work_end: Option<NaiveTime>,
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
        let start = minutes(self.start);
        (start, start + minutes_until(self.start, self.end))
    }

    pub fn duration(&self) -> TimeDelta {
        TimeDelta::minutes(minutes_until(self.start, self.end).into())
    }

    /// Whether the block belongs to a `break` project. Blocks of projects
    /// missing from `projects` are work.
    fn is_break(&self, projects: &[Project]) -> bool {
        projects
            .iter()
            .any(|project| project.slug == self.project && project.is_break())
    }
}

fn minutes(time: NaiveTime) -> u32 {
    time.hour() * 60 + time.minute()
}

/// Minutes from `start` to `end`, where an `end` before `start` lies on the next day.
fn minutes_until(start: NaiveTime, end: NaiveTime) -> u32 {
    let (start, end) = (minutes(start), minutes(end));
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
    work: WorkTime,
    #[serde(default)]
    blocks: Vec<FrontMatterBlock>,
}

#[derive(Default, Deserialize)]
struct WorkTime {
    #[serde(default, deserialize_with = "optional_time")]
    start: Option<NaiveTime>,
    #[serde(default, deserialize_with = "optional_time")]
    end: Option<NaiveTime>,
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

fn optional_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<NaiveTime>, D::Error> {
    time(deserializer).map(Some)
}

impl Day {
    /// Reads a day file. Its date has to match the file name.
    pub fn load(path: &Path) -> Result<(Self, Vec<DayWarning>), ReadError> {
        read_file(path, |text| {
            let (day, warnings) = Self::parse(text)?;
            let file_date = path.file_stem().and_then(|stem| stem.to_str());
            if file_date != Some(day.date.format("%Y-%m-%d").to_string().as_str()) {
                return Err(format!("date {} does not match the file name", day.date));
            }
            Ok((day, warnings))
        })
    }

    fn parse(text: &str) -> Result<(Self, Vec<DayWarning>), String> {
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
        blocks.sort_by_key(|block| block.span());

        let ids: Vec<BlockId> = blocks.iter().map(|block| block.id.clone()).collect();
        let parsed = sections::split(body, front_matter.date, &ids);
        for section in parsed.sections {
            let block = blocks
                .iter_mut()
                .find(|block| block.id == section.id)
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
            work_start: front_matter.work.start,
            work_end: front_matter.work.end,
            blocks,
            note: parsed.note.to_owned(),
            unknown_fields,
        };
        Ok((day, warnings))
    }

    /// The time worked on this day. Blocks of `break` projects are breaks;
    /// blocks of projects missing from `projects` count as work.
    pub fn working_time(&self, projects: &[Project]) -> TimeDelta {
        let sum = |blocks: Vec<&Block>| blocks.iter().map(|block| block.duration()).sum();
        let (breaks, work): (Vec<&Block>, Vec<&Block>) = self
            .blocks
            .iter()
            .partition(|block| block.is_break(projects));
        match (self.work_start, self.work_end) {
            (Some(start), Some(end)) => {
                let total = TimeDelta::minutes(minutes_until(start, end).into());
                // Hand-edited files may hold more breaks than working hours.
                (total - sum(breaks)).max(TimeDelta::zero())
            }
            _ => sum(work),
        }
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

/// Splits a day file into its front matter and the Markdown below it.
///
/// The front matter slice starts with the newline of the opening `---`, so
/// line numbers in YAML errors match the lines of the file.
fn split_front_matter(text: &str) -> Result<(&str, &str), String> {
    let Some(rest) = text.strip_prefix("---") else {
        return Err("the file has to start with a front matter, a line \"---\"".to_owned());
    };
    let mut end = text.len() - rest.len();
    for line in rest.split_inclusive('\n').skip(1) {
        if line.trim_end() == "---" {
            return Ok((&text[3..end], &text[end + line.len()..]));
        }
        end += line.len();
    }
    Err("the front matter is not closed by a line \"---\"".to_owned())
}

fn validate(front_matter: &FrontMatter) -> Result<(), String> {
    if front_matter.format != FORMAT {
        return Err(format!(
            "unsupported format version {}",
            front_matter.format
        ));
    }
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
    use std::path::PathBuf;

    use super::*;

    fn sample_day(date: &str) -> (Day, Vec<DayWarning>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/sample-vault/daily/2026/09")
            .join(format!("{date}.md"));
        Day::load(&path).unwrap()
    }

    fn time(hour: u32, minute: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap()
    }

    fn block<'a>(day: &'a Day, id: &str) -> &'a Block {
        day.blocks
            .iter()
            .find(|block| block.id.as_str() == id)
            .unwrap()
    }

    fn parse_error(text: &str) -> String {
        Day::parse(text).unwrap_err()
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
        assert_eq!(day.work_start, Some(time(8, 0)));
        assert_eq!(day.work_end, Some(time(16, 30)));
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
        assert_eq!(day.work_start, Some(time(8, 45)));
        assert_eq!(day.work_end, Some(time(17, 15)));
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
        assert_eq!((day.work_start, day.work_end), (None, None));

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
        let working_time = |work: &str| {
            let text = format!(
                "---\nformat: 1\ndate: \"2026-01-05\"\n{work}blocks:\n  \
                 - {{ id: \"aaaa\", start: \"21:00\", end: \"23:00\", project: \"pause\" }}\n  \
                 - {{ id: \"bbbb\", start: \"23:00\", end: \"01:00\", project: \"unknown\" }}\n\
                 ---\n"
            );
            Day::parse(&text).unwrap().0.working_time(&projects)
        };
        // Blocks of unknown projects count as work, also past midnight.
        assert_eq!(working_time(""), TimeDelta::hours(2));
        assert_eq!(
            working_time("work: { start: \"20:00\", end: \"02:00\" }\n"),
            TimeDelta::hours(4)
        );
        // More breaks than working hours.
        assert_eq!(
            working_time("work: { start: \"21:00\", end: \"22:00\" }\n"),
            TimeDelta::zero()
        );
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
        let dir = std::env::temp_dir().join(format!("knotbook-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join("2026-01-06.md");
        fs::write(&path, "---\nformat: 1\ndate: \"2026-01-05\"\n---\n").unwrap();
        let err = Day::load(&path).unwrap_err().to_string();
        fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("2026-01-06.md"), "{err}");
        assert!(err.contains("does not match the file name"), "{err}");
    }
}
