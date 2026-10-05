//! Finding what is odd in a vault, for `bitlog doctor`.

use std::fmt;

use chrono::NaiveDate;

use crate::conflict::has_git_markers;
use crate::day::escaped_block_headings;
use crate::error::{ReadError, SaveError};
use crate::markdown::{escape_headings, heading_lines};
use crate::notes::{qualified, wiki_links};
use crate::{
    BlockId, ConflictCopy, Contradiction, Day, DayFile, DayWarning, NotePath, ProjectSlug, Vault,
};

/// Something in a vault that needs a look.
#[derive(Debug)]
pub enum Problem {
    /// A day file or note that cannot be read at all.
    Unreadable(ReadError),
    UnknownProject {
        date: NaiveDate,
        block: BlockId,
        project: ProjectSlug,
    },
    Overlap {
        date: NaiveDate,
        first: BlockId,
        second: BlockId,
    },
    /// A heading in the day note (`block` is `None`) or a block text. This
    /// includes orphaned ID markers and second headings of a block.
    Heading {
        date: NaiveDate,
        block: Option<BlockId>,
        line: String,
    },
    /// The escaped heading of block `id` in the day note (`block` is `None`)
    /// or a block text, as in `\## Title {#id}`. The text below it may
    /// belong to block `id`.
    EscapedHeading {
        date: NaiveDate,
        block: Option<BlockId>,
        id: BlockId,
        line: String,
    },
    /// A conflict copy left by a sync tool, with what keeps it from being
    /// merged into its original.
    Conflict {
        copy: ConflictCopy,
        contradictions: Vec<Contradiction>,
    },
    /// A note with the markers of a failed Git merge. Other files with them
    /// cannot be read.
    GitMarkers(NotePath),
    /// A wiki link in a block text without its project, as in `[[name]]`,
    /// which points nowhere in a day file. It used to point to a note of
    /// the block's project.
    ShortLink {
        date: NaiveDate,
        block: BlockId,
        link: String,
    },
    /// A wiki link in a note, on line `line` counted from 1, that points to
    /// no note, as it is written.
    BrokenLink {
        note: NotePath,
        line: usize,
        link: String,
    },
}

impl Problem {
    /// Whether [`Vault::escape_headings`] solves it.
    pub fn can_be_escaped(&self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    /// Whether [`Vault::qualify_links`] solves it.
    pub fn can_be_qualified(&self) -> bool {
        matches!(self, Self::ShortLink { .. })
    }

    /// Whether [`Vault::merge_conflict`] solves it.
    pub fn can_be_merged(&self) -> bool {
        matches!(self, Self::Conflict { contradictions, .. } if contradictions.is_empty())
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(err) => err.fmt(f),
            Self::UnknownProject {
                date,
                block,
                project,
            } => write!(
                f,
                "{date}: block {block} belongs to the unknown project {project}"
            ),
            Self::Overlap {
                date,
                first,
                second,
            } => write!(f, "{date}: blocks {first} and {second} overlap"),
            Self::Heading {
                date,
                block: Some(block),
                line,
            } => write!(f, "{date}: the text of block {block} has a heading: {line}"),
            Self::Heading {
                date,
                block: None,
                line,
            } => write!(f, "{date}: the day note has a heading: {line}"),
            Self::EscapedHeading {
                date,
                block: Some(block),
                id,
                line,
            } => write!(
                f,
                "{date}: the text of block {block} has an escaped heading of block {id}: {line}"
            ),
            Self::EscapedHeading {
                date,
                block: None,
                id,
                line,
            } => write!(
                f,
                "{date}: the day note has an escaped heading of block {id}: {line}"
            ),
            Self::Conflict {
                copy,
                contradictions,
            } => {
                write!(f, "{}: a sync conflict copy", copy.path.display())?;
                if contradictions.is_empty() {
                    return write!(f, " that can be merged");
                }
                let reasons: Vec<String> = contradictions.iter().map(ToString::to_string).collect();
                write!(f, "; {}", reasons.join(", "))
            }
            Self::ShortLink { date, block, link } => write!(
                f,
                "{date}: the text of block {block} has a wiki link without its project: {link}"
            ),
            Self::GitMarkers(note) => write!(f, "{note}: Git conflict markers"),
            Self::BrokenLink { note, line, link } => {
                write!(f, "{note}:{line}: the wiki link {link} points to no note")
            }
        }
    }
}

impl Vault {
    /// Checks for sync conflict copies, then all day files, oldest first,
    /// then the notes of all projects.
    pub fn check(&self) -> Result<Vec<Problem>, ReadError> {
        let mut problems = Vec::new();
        for copy in self.conflict_copies()? {
            match self.contradictions(&copy) {
                Ok(contradictions) => problems.push(Problem::Conflict {
                    copy,
                    contradictions,
                }),
                Err(err) => problems.push(Problem::Unreadable(err)),
            }
        }
        for date in self.all_days()? {
            match self.load_day(date) {
                Ok(Some(file)) => problems.extend(self.check_day(&file)),
                Ok(None) => {}
                Err(err) => problems.push(Problem::Unreadable(err)),
            }
        }
        for note in self.all_notes() {
            let note = note?;
            match self.load_note(&note) {
                Ok(file) if has_git_markers(&file.text) => {
                    problems.push(Problem::GitMarkers(note));
                }
                Ok(file) => problems.extend(self.broken_links(&note, &file.text)),
                Err(err) => problems.push(Problem::Unreadable(err)),
            }
        }
        Ok(problems)
    }

    /// Escapes the headings in the day note and block texts of the day
    /// `date`, so that they read as plain text.
    pub fn escape_headings(&self, date: NaiveDate) -> Result<(), SaveError> {
        let Some(file) = self.load_day(date)? else {
            return Ok(());
        };
        self.update_day(&file, |day| {
            day.note = escape_headings(&day.note);
            for block in &mut day.blocks {
                block.text = escape_headings(&block.text);
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Names the project of their block in the short wiki links of the
    /// block texts of the day `date`, as in `[[infra/deployment]]`, the
    /// note they used to point to.
    pub fn qualify_links(&self, date: NaiveDate) -> Result<(), SaveError> {
        let Some(file) = self.load_day(date)? else {
            return Ok(());
        };
        self.update_day(&file, |day| {
            for block in &mut day.blocks {
                if let Some(text) = qualified(&block.text, &block.project) {
                    block.text = text;
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    fn check_day(&self, file: &DayFile) -> Vec<Problem> {
        let day = &file.day;
        let date = day.date;
        let mut problems: Vec<Problem> = day
            .blocks
            .iter()
            .filter(|block| self.project(&block.project).is_none())
            .map(|block| Problem::UnknownProject {
                date,
                block: block.id.clone(),
                project: block.project.clone(),
            })
            .collect();
        // Marker warnings show up as headings below.
        problems.extend(file.warnings.iter().filter_map(|warning| match warning {
            DayWarning::Overlap { first, second } => Some(Problem::Overlap {
                date,
                first: first.clone(),
                second: second.clone(),
            }),
            _ => None,
        }));
        problems.extend(headings(day));
        problems.extend(escaped_headings(day));
        problems.extend(short_links(day));
        problems
    }

    fn broken_links(&self, note: &NotePath, text: &str) -> Vec<Problem> {
        wiki_links(text, Some(note.project()))
            .into_iter()
            .filter(|link| {
                !link
                    .note
                    .as_ref()
                    .is_some_and(|target| self.note_path(target).is_file())
            })
            .map(|link| Problem::BrokenLink {
                note: note.clone(),
                line: text[..link.span.start].matches('\n').count() + 1,
                link: text[link.span].to_owned(),
            })
            .collect()
    }
}

/// The short wiki links in the block texts of `day`. Those in the day note
/// never pointed anywhere.
fn short_links(day: &Day) -> Vec<Problem> {
    day.blocks
        .iter()
        .flat_map(|block| {
            wiki_links(&block.text, Some(&block.project))
                .into_iter()
                .filter(|link| link.note.is_some() && link.is_short(&block.text))
                .map(|link| Problem::ShortLink {
                    date: day.date,
                    block: block.id.clone(),
                    link: block.text[link.span].to_owned(),
                })
        })
        .collect()
}

fn headings(day: &Day) -> Vec<Problem> {
    texts(day)
        .flat_map(|(block, text)| {
            heading_lines(text)
                .into_iter()
                .map(move |line| Problem::Heading {
                    date: day.date,
                    block: block.cloned(),
                    line: text[line].trim().to_owned(),
                })
        })
        .collect()
}

fn escaped_headings(day: &Day) -> Vec<Problem> {
    let ids: Vec<BlockId> = day.blocks.iter().map(|block| block.id.clone()).collect();
    texts(day)
        .flat_map(|(block, text)| {
            escaped_block_headings(text, &ids)
                .into_iter()
                .map(move |(id, line)| Problem::EscapedHeading {
                    date: day.date,
                    block: block.cloned(),
                    id,
                    line: line.to_owned(),
                })
        })
        .collect()
}

/// The day note (without a block) and the block texts of `day`.
fn texts(day: &Day) -> impl Iterator<Item = (Option<&BlockId>, &String)> {
    std::iter::once((None, &day.note)).chain(
        day.blocks
            .iter()
            .map(|block| (Some(&block.id), &block.text)),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::file::sample_copy;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    fn messages(vault: &Vault) -> Vec<String> {
        vault
            .check()
            .unwrap()
            .iter()
            .map(|problem| problem.to_string())
            .collect()
    }

    #[test]
    fn sample_vault() {
        let (_dir, vault) = sample_copy();
        assert_eq!(
            messages(&vault),
            [
                "daily/2026/09/2026-09-22.sync-conflict-20260922-181530-KNOTBK7.md: \
                 a sync conflict copy; the day note differs",
                "2026-09-22: the text of block t5u6 has a heading: ## Root cause",
                "2026-09-23: the text of block cc33 has a heading: ## Old notes {#zz99}",
            ]
        );
    }

    #[test]
    fn broken_wiki_links() {
        let (_dir, vault) = sample_copy();
        let note = vault
            .create_note(&"infra".parse().unwrap(), "Links", date(26))
            .unwrap();
        fs::write(
            vault.note_path(&note),
            "[[deployment]] [[Missing]]\n\n`[[in code]]`\n\
             [[webshop/checkout-flow#Payment|pay]] [[a/b/c]]\n",
        )
        .unwrap();
        assert_eq!(
            messages(&vault)[3..],
            [
                "projects/infra/notes/Links.md:1: the wiki link [[Missing]] points to no note",
                "projects/infra/notes/Links.md:4: the wiki link [[a/b/c]] points to no note",
            ]
        );
    }

    #[test]
    fn short_links_get_their_project() {
        let (_dir, vault) = sample_copy();
        let file = vault.load_day(date(21)).unwrap().unwrap();
        vault
            .update_day(&file, |day| {
                day.note = "[[deployment]]".to_owned();
                // Block m1n2 belongs to infra.
                day.blocks[5].text = "[[deployment#Steps|how]] [[Missing]] \
                                      [[webshop/checkout-flow]] `[[code]]` [[a/b/c]]"
                    .to_owned();
                Ok(())
            })
            .unwrap();
        let problems = vault.check().unwrap();
        let short: Vec<String> = problems
            .iter()
            .filter(|problem| problem.can_be_qualified())
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            short,
            [
                "2026-09-21: the text of block m1n2 has a wiki link without its project: \
                 [[deployment#Steps|how]]",
                "2026-09-21: the text of block m1n2 has a wiki link without its project: \
                 [[Missing]]",
            ]
        );

        vault.qualify_links(date(21)).unwrap();
        let day = vault.load_day(date(21)).unwrap().unwrap().day;
        assert_eq!(day.note, "[[deployment]]");
        assert_eq!(
            day.blocks[5].text,
            "[[infra/deployment#Steps|how]] [[infra/Missing]] \
             [[webshop/checkout-flow]] `[[code]]` [[a/b/c]]"
        );
        assert!(!vault.check().unwrap().iter().any(Problem::can_be_qualified));
    }

    #[test]
    fn escaped_block_headings() {
        let (_dir, vault) = sample_copy();
        let file = vault.load_day(date(21)).unwrap().unwrap();
        vault
            .update_day(&file, |day| {
                day.note = "\\## {#zz99}".to_owned();
                day.blocks[0].text = "Mails\n\n\\## Review {#c3d4}  \n\nMoved.".to_owned();
                Ok(())
            })
            .unwrap();
        let escaped: Vec<String> = vault
            .check()
            .unwrap()
            .iter()
            .filter(|problem| matches!(problem, Problem::EscapedHeading { .. }))
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            escaped,
            [
                "2026-09-21: the text of block a1b2 has an escaped heading of block c3d4: \
              \\## Review {#c3d4}"
            ]
        );
    }

    #[test]
    fn git_markers() {
        let (_dir, vault) = sample_copy();
        let note = vault.note_path(&"projects/infra/notes/deployment.md".parse().unwrap());
        fs::write(&note, "<<<<<<< HEAD\nOurs\n=======\nTheirs\n>>>>>>> main\n").unwrap();
        fs::write(
            vault.tasks_path(),
            "<<<<<<< HEAD\nformat = 1\n=======\n>>>>>>> main\n",
        )
        .unwrap();
        let tasks = vault.load_tasks().unwrap_err().to_string();
        assert!(
            tasks.ends_with("tasks.toml: it holds Git conflict markers, resolve them with Git")
        );
        assert_eq!(
            messages(&vault).last().unwrap(),
            "projects/infra/notes/deployment.md: Git conflict markers"
        );
    }

    #[test]
    fn escaping_leaves_only_what_needs_a_person() {
        let (_dir, vault) = sample_copy();
        let path = vault.day_path(date(25));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "---\nformat: 1\ndate: \"2026-09-25\"\nblocks:\n\
             - { id: \"aa11\", start: \"09:00\", end: \"10:00\", project: \"gone\" }\n\
             - { id: \"bb22\", start: \"09:30\", end: \"11:00\", project: \"infra\" }\n\
             ---\n\n# 2026-09-25\n\nNote\n===\n\n## {#aa11}\n\n## {#bb22}\n",
        )
        .unwrap();
        // Not a day file, so not checked.
        fs::write(path.with_file_name("notes.md"), "# Heading\n").unwrap();
        // Unreadable.
        fs::write(vault.day_path(date(26)), "no front matter").unwrap();

        let problems = vault.check().unwrap();
        let escapable: Vec<String> = problems
            .iter()
            .filter(|problem| problem.can_be_escaped())
            .map(|problem| problem.to_string())
            .collect();
        assert_eq!(escapable.len(), 3, "{escapable:?}");
        assert!(escapable.contains(&"2026-09-25: the day note has a heading: Note".to_owned()));

        for date in [date(22), date(23), date(25), date(26)] {
            let _ = vault.escape_headings(date);
        }
        let remaining = messages(&vault);
        assert_eq!(remaining.len(), 4, "{remaining:?}");
        assert_eq!(
            remaining[1..3],
            [
                "2026-09-25: block aa11 belongs to the unknown project gone",
                "2026-09-25: blocks aa11 and bb22 overlap",
            ]
        );
        assert!(remaining[3].starts_with("invalid "), "{remaining:?}");
        // Escaped, the orphaned marker no longer warns either.
        let day = vault.load_day(date(23)).unwrap().unwrap();
        assert!(day.warnings.is_empty(), "{:?}", day.warnings);
        assert!(day.day.blocks[2].text.contains("\\## Old notes {#zz99}"));
    }
}
