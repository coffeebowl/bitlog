//! What the help explains, topic by topic: a figure atop each page, then
//! running text under headings, with rows where things are looked up rather
//! than read, like Markdown syntax. Keep it in step with the app.

use gettextrs::gettext;

use super::MARKDOWN;
use super::figures::Kind;

pub(super) struct Topic {
    /// Of its page in the navigation view.
    pub tag: &'static str,
    pub title: String,
    pub blocks: Vec<Block>,
}

pub(super) enum Block {
    Heading(String),
    /// A paragraph, in Pango markup.
    Text(String),
    Rows(Vec<Row>),
    /// Shown as it is, in a monospace font.
    Code(String),
    /// A part of the app drawn as an example.
    Figure(Kind),
}

pub(super) struct Row {
    pub title: String,
    pub subtitle: Option<String>,
    /// Markdown, shown in a monospace font.
    pub example: Option<String>,
    /// An accelerator, like `<Control>b`.
    pub keys: Option<&'static str>,
}

impl Row {
    fn about(mut self, subtitle: String) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    fn example(mut self, example: impl Into<String>) -> Self {
        self.example = Some(example.into());
        self
    }

    fn keys(mut self, keys: &'static str) -> Self {
        self.keys = Some(keys);
        self
    }
}

fn row(title: String) -> Row {
    Row {
        title,
        subtitle: None,
        example: None,
        keys: None,
    }
}

/// All topics, in the order the help lists them.
pub(super) fn all() -> Vec<Topic> {
    vec![days(), projects(), tasks(), reports(), markdown(), vault()]
}

fn days() -> Topic {
    Topic {
        tag: "days",
        title: gettext("Days and Blocks"),
        blocks: vec![
            Block::Figure(Kind::Day),
            Block::Text(gettext(
                "Your day is made of blocks. Each block records which project you worked on and when, for example from 9:00 to 10:30. You don’t need to log as you go: filling in the day afterwards works fine, and quarter hours are precise enough. You can change the grid in the preferences.",
            )),
            Block::Heading(gettext("Work, Overhead and Breaks")),
            Block::Text(gettext(
                "Every project belongs to one of three categories. <b>Work</b> is the actual work, such as a customer project. <b>Overhead</b> is everything around it that still counts as working time: meetings, e-mails, chats, research. <b>Break</b> is time off and doesn’t count, just like time without a block. Unlike a gap, though, a break is a block, so it can have a title and notes: “Lunch”, “Shopping”, “Talk with John”.",
            )),
            Block::Text(gettext(
                "A new vault comes with the projects Meetings and Filler for overhead and Break for breaks. Filler is for anything that fits nowhere else.",
            )),
            Block::Heading(gettext("Logging Blocks")),
            Block::Text(gettext(
                "To add a block, drag over free time in the timeline or press <b>Ctrl+N</b>. Drag a block to move it, or drag its top or bottom edge to change its length. Click a block to change its project, title or text. On touch screens, press and hold instead of dragging.",
            )),
            Block::Heading(gettext("The Day")),
            Block::Text(gettext(
                "To start an empty day, click <b>Log Day</b>. Besides its blocks, a day has a note for anything about the day as a whole, a kind of day and a location. Vacation, sick days and public holidays reduce the target hours of their week.",
            )),
            Block::Text(gettext(
                "Your open tasks are shown on every day. The <b>Standup</b> button summarizes your last working day and today, including your commits, ready to paste into your team chat.",
            )),
        ],
    }
}

fn projects() -> Topic {
    Topic {
        tag: "projects",
        title: gettext("Projects and Notes"),
        blocks: vec![
            Block::Figure(Kind::Projects),
            Block::Text(gettext(
                "Besides a category, each project has a color and a status. Pause a project while it’s on hold and archive it once it’s finished; archived projects no longer appear when you choose a project for a block.",
            )),
            Block::Heading(gettext("Notes")),
            Block::Text(gettext(
                "Each project can have notes in Markdown. To link to another note, write <tt>[[project/note]]</tt>, or just <tt>[[note]]</tt> within the same project. Clicking a link to a note that doesn’t exist yet creates it, a quick way to start a new note. The links button above a note shows which notes and days link to it.",
            )),
            Block::Heading(gettext("Files")),
            Block::Text(gettext(
                "You can also keep files with a project, such as PDFs, office documents or images. They open in your system’s default app.",
            )),
            Block::Heading(gettext("Git")),
            Block::Text(gettext(
                "If a project has a Git repository on this computer, add its folder under <b>Edit Project</b>. The project then shows its commits, and the standup includes yours. The folder is stored per computer, since it’s usually different on each one.",
            )),
        ],
    }
}

fn tasks() -> Topic {
    Topic {
        tag: "tasks",
        title: gettext("Tasks"),
        blocks: vec![
            Block::Figure(Kind::Tasks),
            Block::Text(gettext(
                "The task list is for small things you shouldn’t forget, like getting back to someone. It isn’t tied to a project or a day: open tasks show up every day until you check them off, so nothing needs to be carried over.",
            )),
            Block::Text(gettext(
                "Tasks with a due date turn red once they’re overdue. Drag tasks to reorder them, or use <b>Alt+Up</b> and <b>Alt+Down</b>. <b>Archive</b> moves finished tasks out of the list into a file for each year.",
            )),
        ],
    }
}

fn reports() -> Topic {
    Topic {
        tag: "reports",
        title: gettext("Calendar and Reports"),
        blocks: vec![
            Block::Figure(Kind::Week),
            Block::Text(gettext(
                "The calendar shows how long you worked each day of a month, or a week as a chart. The reports break down a week, month or year by project and category. Target hours are set in the preferences and divided among your workdays.",
            )),
            Block::Heading(gettext("Export")),
            Block::Text(gettext(
                "From the reports, you can export your blocks or your remote work days as CSV, for a timesheet for example, or a week as a Markdown report. Exports are saved in the exports folder of the vault.",
            )),
        ],
    }
}

fn markdown() -> Topic {
    Topic {
        tag: MARKDOWN,
        title: gettext("Markdown"),
        blocks: vec![
            Block::Figure(Kind::Markdown),
            Block::Text(gettext(
                "Project notes, day notes and block texts are written in Markdown with a live preview: text is formatted as you type, and the markers only show while the cursor is on them.",
            )),
            Block::Heading(gettext("Notes, Days and Blocks")),
            Block::Text(gettext(
                "Project notes support everything below. Day notes and block texts have no headings, because the day file already uses headings for its blocks; a <tt>#</tt> stays plain text there. Links to notes always include the project there, as in <tt>[[project/note]]</tt>. In a project note, <tt>[[note]]</tt> is enough for a note in the same project.",
            )),
            Block::Heading(gettext("Text")),
            Block::Text(gettext(
                "Select text and type <tt>*</tt>, <tt>_</tt> or <tt>`</tt> to wrap it.",
            )),
            Block::Rows(vec![
                row(gettext("Heading"))
                    .about(gettext("Only in project notes"))
                    .example(gettext("# Heading")),
                row(gettext("Bold"))
                    .example(gettext("**bold**"))
                    .keys("<Control>b"),
                row(gettext("Italic"))
                    .example(gettext("*italic*"))
                    .keys("<Control>i"),
                row(gettext("Code"))
                    .example(gettext("`code`"))
                    .keys("<Control>e"),
                row(gettext("Strikethrough")).example(gettext("~~struck~~")),
            ]),
            Block::Heading(gettext("Lists")),
            Block::Rows(vec![
                row(gettext("Bullet List")).example(gettext("- item")),
                row(gettext("Numbered List"))
                    .about(gettext("Renumbered after Enter and Tab"))
                    .example(gettext("1. item")),
                row(gettext("Task"))
                    .about(gettext("Click the box to check it off"))
                    .example(gettext("- [ ] task"))
                    .keys("<Control>l"),
                row(gettext("Indent"))
                    .about(gettext("Shift+Tab moves the item back out"))
                    .keys("Tab"),
            ]),
            Block::Heading(gettext("Links and Images")),
            Block::Rows(vec![
                row(gettext("Link"))
                    .about(gettext("Or paste a web address over selected text"))
                    // No gettext: xgettext warns about URLs.
                    .example("[text](https://…)"),
                row(gettext("Note Link"))
                    .about(gettext("Notes are suggested after [["))
                    .example(gettext("[[project/note]]")),
                row(gettext("Follow Link"))
                    .about(gettext("Or click it"))
                    .keys("<Control>Return"),
                row(gettext("Image"))
                    .about(gettext(
                        "Or paste or drop an image to copy it into the vault",
                    ))
                    .example(gettext("![](photo.png)")),
            ]),
            Block::Heading(gettext("Blocks")),
            Block::Rows(vec![
                row(gettext("Quote")).example(gettext("> quote")),
                row(gettext("Callout"))
                    .about(gettext("Also TIP, IMPORTANT, WARNING and CAUTION"))
                    .example("> [!NOTE]"),
                row(gettext("Code Block"))
                    .about(gettext(
                        "Name a language to highlight it, or mermaid for a diagram",
                    ))
                    .example("```rust"),
                row(gettext("Table"))
                    .about(gettext("Enter adds a row"))
                    .example("| a | b |"),
                row(gettext("Rule")).example("---"),
            ]),
        ],
    }
}

fn vault() -> Topic {
    Topic {
        tag: "vault",
        title: gettext("Vault and Files"),
        blocks: vec![
            Block::Code(VAULT_TREE.to_owned()),
            Block::Text(gettext(
                "All data is stored in plain text files in an ordinary folder, the vault. They can be read and changed in any editor, and synced with any tool or kept in Git.",
            )),
            Block::Text(gettext(
                "<tt>bitlog.toml</tt> holds the settings shared by all devices. <tt>.bitlog/</tt> holds the search index and the repository folders of this device; do not sync it. A new vault leaves it out of Git already.",
            )),
            Block::Heading(gettext("Day Files")),
            Block::Text(gettext(
                "Each day is a Markdown file that reads well in any editor. The YAML at the top holds the kind of day, the location and the blocks with their times and projects. Below follow the date, the day note and a heading for each block with its text.",
            )),
            Block::Code(day_file()),
            Block::Text(gettext(
                "BitLog writes the YAML anew on every save, but keeps your texts as they are. Headings typed into a day note or block text are saved with a backslash, as in <tt>\\# Text</tt>, so that they stay text.",
            )),
            Block::Heading(gettext("Sync")),
            Block::Text(gettext(
                "BitLog notices changes made elsewhere and shows them right away. When a sync tool leaves conflict copies, BitLog merges those that do not contradict the original; the others wait in the sidebar to be resolved. Conflict markers of Git are resolved with Git.",
            )),
        ],
    }
}

const VAULT_TREE: &str = "\
my-vault/
├─ bitlog.toml
├─ tasks.toml
├─ daily/2026/10/2026-10-06.md
├─ projects/webshop/
│  ├─ project.toml
│  ├─ notes/
│  └─ assets/
├─ images/
├─ exports/
└─ .bitlog/";

/// A short day file, with its texts translated.
fn day_file() -> String {
    format!(
        "---
format: 1
date: \"2026-10-06\"
kind: \"work\"
location: \"remote\"
blocks:
  - {{ id: \"k7f3\", start: \"09:00\", end: \"10:30\", project: \"webshop\" }}
---

# 2026-10-06

{note}

## {title} {{#k7f3}}

{text}",
        note = gettext("A note about the whole day."),
        title = gettext("Code review"),
        text = gettext("The text of the block."),
    )
}
