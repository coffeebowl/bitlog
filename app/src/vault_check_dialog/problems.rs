//! What checking the vault finds, by kind and described for the user, with
//! where to see it and what the app can do about it.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use bitlog_core::{
    BlockId, Day, ImageProblem, NotePath, Problem, ReadError, SaveError, Vault, VaultChange,
};
use chrono::NaiveDate;
use gettextrs::{gettext, ngettext};
use gtk::glib::markup_escape_text as escape;

use crate::format::{format_full_date, plural, title_markup};
use crate::sync_conflict_dialog::file_title;

/// The kinds of problems, in the order the dialog shows them: sync
/// conflicts, which the sidebar shows, then the severe ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Conflict,
    Unreadable,
    GitMarkers,
    EscapedHeading,
    Overlap,
    UnknownProject,
    Heading,
    ShortLink,
    BrokenLink,
    BadImage,
    UnusedImage,
    DuplicateImages,
}

impl Kind {
    pub const ALL: [Self; 12] = [
        Self::Conflict,
        Self::Unreadable,
        Self::GitMarkers,
        Self::EscapedHeading,
        Self::Overlap,
        Self::UnknownProject,
        Self::Heading,
        Self::ShortLink,
        Self::BrokenLink,
        Self::BadImage,
        Self::UnusedImage,
        Self::DuplicateImages,
    ];

    fn of(problem: &Problem) -> Self {
        match problem {
            Problem::Unreadable(_) => Self::Unreadable,
            Problem::GitMarkers(_) => Self::GitMarkers,
            Problem::EscapedHeading { .. } => Self::EscapedHeading,
            Problem::Overlap { .. } => Self::Overlap,
            Problem::UnknownProject { .. } => Self::UnknownProject,
            Problem::Conflict { .. } => Self::Conflict,
            Problem::Heading { .. } => Self::Heading,
            Problem::ShortLink { .. } => Self::ShortLink,
            Problem::BrokenLink { .. } => Self::BrokenLink,
            Problem::BadImage { .. } | Problem::BadNoteImage { .. } => Self::BadImage,
            Problem::UnusedImage(_) | Problem::UnusedImagesUnchecked => Self::UnusedImage,
            Problem::DuplicateImages(_) => Self::DuplicateImages,
        }
    }

    /// Whether problems of this kind are reported as soon as the vault is
    /// opened: data is left out or may be in the wrong place. Sync
    /// conflicts are not, the sidebar shows them already.
    pub fn is_severe(self) -> bool {
        matches!(
            self,
            Self::Unreadable
                | Self::GitMarkers
                | Self::EscapedHeading
                | Self::Overlap
                | Self::UnknownProject
        )
    }

    pub fn title(self) -> String {
        match self {
            Self::Unreadable => gettext("Unreadable Files"),
            Self::GitMarkers => gettext("Git Conflict Markers"),
            Self::EscapedHeading => gettext("Escaped Block Headings"),
            Self::Overlap => gettext("Overlapping Blocks"),
            Self::UnknownProject => gettext("Unknown Projects"),
            Self::Conflict => gettext("Sync Conflicts"),
            Self::Heading => gettext("Headings in Texts"),
            Self::ShortLink => gettext("Links Without Project"),
            Self::BrokenLink => gettext("Broken Links"),
            Self::BadImage => gettext("Image Links"),
            Self::UnusedImage => gettext("Unused Images"),
            Self::DuplicateImages => gettext("Duplicate Images"),
        }
    }

    /// What problems of this kind mean and how to solve them.
    pub fn description(self) -> String {
        match self {
            Self::Unreadable => gettext(
                "These files cannot be read, for example because their front matter is broken. BitLog leaves them out of the calendar, reports and search until they are corrected in a text editor.",
            ),
            Self::GitMarkers => gettext(
                "A Git merge stopped with a conflict and left its markers in these notes. Resolve the conflict with Git, or remove the markers and the version you don’t want by hand.",
            ),
            Self::EscapedHeading => gettext(
                "A block heading saved with a backslash, as in \\## Title {#id}, stands in another text. The text below it may belong to that block, for example after a code block was left open while editing elsewhere. Move the text to its block and delete the heading.",
            ),
            Self::Overlap => {
                gettext("These blocks share time. Shorten or move one of them in the timeline.")
            }
            Self::UnknownProject => gettext(
                "These blocks belong to a project the vault doesn’t have, perhaps because its folder was removed or renamed. Add the project again, or choose another one for the block.",
            ),
            Self::Conflict => gettext(
                "These files were changed on two devices at once, and the versions contradict each other. Open one to choose what to keep. Conflicts without contradictions are merged automatically. The sidebar shows them, too.",
            ),
            Self::Heading => gettext(
                "Headings typed into a day note or block text in another editor. These texts have no headings: saved with a backslash, as in \\# Text, they stay text. Fix All does that for all of them; editing a text in BitLog does it, too.",
            ),
            Self::ShortLink => gettext(
                "In a day, a wiki link needs its project, as in [[project/note]], or it points nowhere. These links used to point to a note of the block’s project. Fix All adds that project to all of them.",
            ),
            Self::BrokenLink => gettext(
                "These wiki links in notes point to no note. Correct the link, or click it in the note to create the note.",
            ),
            Self::BadImage => gettext(
                "These images are missing, or they lie outside the vault, where BitLog doesn’t show them and other devices don’t find them. Paste or drop the image into the text again, or remove the link.",
            ),
            Self::UnusedImage => gettext(
                "No text shows these images. Delete them in the file manager if they’re not needed.",
            ),
            Self::DuplicateImages => gettext(
                "These images have the same content. Keep one, change the links to the others, then delete them.",
            ),
        }
    }

    /// Asks whether to fix all `count` problems of this kind, which the
    /// app can fix.
    pub fn fix_all_question(self, count: usize) -> String {
        let question = match self {
            Self::Heading => ngettext(
                "Save the {count} heading with a backslash, so that it stays text?",
                "Save the {count} headings with a backslash, so that they stay text?",
                plural(count),
            ),
            Self::ShortLink => ngettext(
                "Add the project of its block to the {count} wiki link?",
                "Add the project of their block to the {count} wiki links?",
                plural(count),
            ),
            _ => unreachable!("only headings and links are fixed"),
        };
        question.replace("{count}", &count.to_string())
    }
}

/// A problem as the dialog lists it.
#[derive(Debug)]
pub struct Entry {
    /// Pango markup.
    pub title: String,
    /// Pango markup, empty if there is nothing more to say.
    pub subtitle: String,
    pub place: Place,
    pub fix: Option<Fix>,
}

/// Where the user sees a problem, to solve it there.
#[derive(Debug, Clone)]
pub enum Place {
    Day(NaiveDate),
    Block(NaiveDate, BlockId),
    Note(NotePath),
    /// A sync conflict copy, relative to the vault, resolved in its dialog.
    Conflict(PathBuf),
    /// A file shown in its folder, as the app cannot show it.
    File(PathBuf),
}

/// What the app can do about a problem itself. It solves all problems of
/// that kind on the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    EscapeHeadings(NaiveDate),
    QualifyLinks(NaiveDate),
}

impl Fix {
    pub fn apply(self, vault: &Vault) -> Result<(), SaveError> {
        match self {
            Self::EscapeHeadings(date) => vault.escape_headings(date),
            Self::QualifyLinks(date) => vault.qualify_links(date),
        }
    }
}

/// What checking the vault found.
#[derive(Debug)]
pub struct Report {
    /// All kinds in the order to show them, with their problems.
    pub groups: Vec<(Kind, Vec<Entry>)>,
    /// Whether unused images were not looked for, as some texts could not
    /// be read.
    pub unused_images_unchecked: bool,
}

impl Report {
    /// How many of the problems found are severe.
    pub fn severe(&self) -> usize {
        self.groups
            .iter()
            .filter(|(kind, _)| kind.is_severe())
            .map(|(_, entries)| entries.len())
            .sum()
    }
}

/// Checks `vault` for problems of all kinds.
pub fn check(vault: &Vault) -> Result<Report, ReadError> {
    let mut groups: BTreeMap<Kind, Vec<Entry>> = Kind::ALL
        .into_iter()
        .map(|kind| (kind, Vec::new()))
        .collect();
    let mut unused_images_unchecked = false;
    let mut days = Days {
        vault,
        read: HashMap::new(),
    };
    for problem in vault.check()? {
        if let Problem::UnusedImagesUnchecked = problem {
            unused_images_unchecked = true;
            continue;
        }
        groups
            .get_mut(&Kind::of(&problem))
            .expect("all kinds have a group")
            .push(days.describe(problem));
    }
    Ok(Report {
        groups: groups.into_iter().collect(),
        unused_images_unchecked,
    })
}

/// The days problems were found in, read once each for the titles of
/// their blocks.
struct Days<'a> {
    vault: &'a Vault,
    read: HashMap<NaiveDate, Option<Day>>,
}

impl Days<'_> {
    fn describe(&mut self, problem: Problem) -> Entry {
        let vault = self.vault;
        match problem {
            Problem::Unreadable(err) => {
                // The path is the title; YAML errors go on with an excerpt
                // of the file.
                let (path, reason) = match err {
                    ReadError::Io { path, source } => (path, source.to_string()),
                    ReadError::Invalid { path, message } => {
                        let first = message.lines().next().unwrap_or_default().to_owned();
                        (path, first)
                    }
                };
                let title = path.strip_prefix(vault.root()).unwrap_or(&path);
                Entry {
                    title: escape(&title.to_string_lossy()).into(),
                    subtitle: escape(&reason).into(),
                    place: Place::File(path),
                    fix: None,
                }
            }
            Problem::GitMarkers(note) => note_entry(vault, note, String::new()),
            Problem::EscapedHeading {
                date, block, line, ..
            } => self.text_entry(date, block, &line, None),
            Problem::Overlap {
                date,
                first,
                second,
            } => {
                let subtitle = gettext("{first} and {second}")
                    .replace("{first}", &self.block_title(date, &first))
                    .replace("{second}", &self.block_title(date, &second));
                Entry {
                    title: escape(&format_full_date(date)).into(),
                    subtitle,
                    place: Place::Block(date, first),
                    fix: None,
                }
            }
            Problem::UnknownProject {
                date,
                block,
                project,
            } => {
                let subtitle =
                    gettext("Project: {project}").replace("{project}", &escape(project.as_str()));
                Entry {
                    title: self.place_title(date, Some(&block)),
                    subtitle,
                    place: Place::Block(date, block),
                    fix: None,
                }
            }
            Problem::Conflict {
                copy,
                contradictions,
            } => {
                let subtitle = if contradictions.is_empty() {
                    gettext("No contradictions")
                } else {
                    ngettext(
                        "{count} contradiction",
                        "{count} contradictions",
                        plural(contradictions.len()),
                    )
                    .replace("{count}", &contradictions.len().to_string())
                };
                Entry {
                    title: escape(&file_title(vault, &copy.of)).into(),
                    subtitle,
                    place: Place::Conflict(copy.path),
                    fix: None,
                }
            }
            Problem::Heading { date, block, line } => {
                self.text_entry(date, block, &line, Some(Fix::EscapeHeadings(date)))
            }
            Problem::ShortLink { date, block, link } => {
                self.text_entry(date, Some(block), &link, Some(Fix::QualifyLinks(date)))
            }
            Problem::BrokenLink { note, line, link } => {
                note_entry(vault, note, on_line(line, &link))
            }
            Problem::BadImage {
                date,
                block,
                image,
                problem,
            } => self.text_entry(date, block, &image_problem(problem, &image), None),
            Problem::BadNoteImage {
                note,
                line,
                image,
                problem,
            } => note_entry(vault, note, on_line(line, &image_problem(problem, &image))),
            Problem::UnusedImage(image) => file_entry(vault, &image, String::new()),
            Problem::UnusedImagesUnchecked => unreachable!("check notes it in the report"),
            Problem::DuplicateImages(images) => {
                let others = gettext("Same as: {images}")
                    .replace("{images}", &escape(&images[1..].join(", ")));
                file_entry(vault, &images[0], others)
            }
        }
    }

    /// A problem in the day note (`block` is `None`) or a block text,
    /// with `detail`, as written, below.
    fn text_entry(
        &mut self,
        date: NaiveDate,
        block: Option<BlockId>,
        detail: &str,
        fix: Option<Fix>,
    ) -> Entry {
        Entry {
            title: self.place_title(date, block.as_ref()),
            subtitle: escape(detail).into(),
            place: match block {
                Some(block) => Place::Block(date, block),
                None => Place::Day(date),
            },
            fix,
        }
    }

    /// The day and the block, or the day note, as in
    /// "September 22, 2026 · Login form".
    fn place_title(&mut self, date: NaiveDate, block: Option<&BlockId>) -> String {
        let text = match block {
            Some(block) => self.block_title(date, block),
            None => escape(&gettext("Day Note")).into(),
        };
        format!("{} · {text}", escape(&format_full_date(date)))
    }

    /// The title of the block `id` as Pango markup, or its ID if the day
    /// lacks it.
    fn block_title(&mut self, date: NaiveDate, id: &BlockId) -> String {
        let vault = self.vault;
        let day = self.read.entry(date).or_insert_with(|| {
            // It was read just now, while checking.
            vault.load_day(date).ok().flatten().map(|file| file.day)
        });
        match day.as_ref().and_then(|day| day.block(id)) {
            Some(block) => title_markup(&block.title, vault.project_name(&block.project)),
            None => escape(id.as_str()).into(),
        }
    }
}

fn note_entry(vault: &Vault, note: NotePath, subtitle: String) -> Entry {
    Entry {
        title: escape(&file_title(vault, &VaultChange::Note(note.clone()))).into(),
        subtitle,
        place: Place::Note(note),
        fix: None,
    }
}

/// An image, relative to the vault, with `subtitle` below.
fn file_entry(vault: &Vault, image: &str, subtitle: String) -> Entry {
    Entry {
        title: escape(image).into(),
        subtitle,
        place: Place::File(vault.root().join(image)),
        fix: None,
    }
}

/// `text` of a note on `line`, as Pango markup.
fn on_line(line: usize, text: &str) -> String {
    gettext("Line {line}: {text}")
        .replace("{line}", &line.to_string())
        .replace("{text}", &escape(text))
}

fn image_problem(problem: ImageProblem, image: &str) -> String {
    match problem {
        ImageProblem::Missing => gettext("Missing: {image}"),
        ImageProblem::OutsideVault => gettext("Outside the vault: {image}"),
    }
    .replace("{image}", image)
}
