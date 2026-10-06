# BitLog file format

**Status:** Format version 1. BitLog implements all of it, except for two
parts marked below: tags are defined but not evaluated yet, and project tasks
are reserved for a later extension.

A BitLog vault is an ordinary folder of plain text files. Every file can be
read and corrected by hand, in any text editor, without BitLog. The files are
the only source of truth; the local index and caches can be rebuilt from them
at any time.

## Terms

| Term | Meaning | Stored in |
| --- | --- | --- |
| Vault | The folder holding all data | `bitlog.toml` at its root |
| Day | One calendar day of work | `daily/YYYY/MM/YYYY-MM-DD.md` |
| Day note | Free text about the whole day | Day file, below the date heading |
| Block | A time span of a day assigned to one project | Day file, front matter and one section |
| Block text | Free text about one block | Day file, below the block heading |
| Project | Something time is spent on | `projects/<slug>/project.toml` |
| Project note | A Markdown note belonging to a project | `projects/<slug>/notes/*.md` |
| Project asset | Any other file kept with a project, such as a PDF or an image | `projects/<slug>/assets/**` |
| Image | A picture shown in day notes, block texts and project notes | `images/**` |
| Task | An item on the global task list | `tasks.toml` |

A block refers to its project by slug. A project note belongs to its project
only through the folder it lives in. There are no references between blocks
and project notes.

## Identifiers

| Identifier | Rule | Example |
| --- | --- | --- |
| Project slug, location key | lowercase letters `a-z`, digits and single hyphens, not at the start or end | `project-a` |
| Block ID, task ID | exactly 4 characters from `a-z` and `0-9` | `k7f3` |
| Note name | the file name without `.md`; not empty, no `/`, `\` or control characters, not starting with `.`, not a sync conflict copy | `Auth middleware` |

A project note is identified by its path relative to the vault,
`projects/<slug>/notes/<name>.md`, always written with `/`. So is a project
asset, `projects/<slug>/assets/<path>`, where every part of `<path>` is a
file or folder name that is not empty, holds no `\` and does not start
with `.`.

## Folder structure

```
my-vault/
  README.md                          # explains the format to humans and LLMs
  bitlog.toml
  tasks.toml                         # global task list
  tasks-archive-2026.toml            # tasks finished in 2026, once archived
  daily/2026/09/2026-09-23.md        # one day: front matter, day note, blocks with text
  projects/project-a/project.toml
  projects/project-a/notes/auth-middleware.md
  projects/project-a/assets/scans/offer.pdf  # any files, in any subfolders
  images/login-form.png              # images shown in texts
  templates/note.md
  exports/                           # generated, may be overwritten
  .bitlog/                         # local, never sync
    index.sqlite
    device.toml                      # per device, e.g. repository paths
```

A new vault is created in a folder that is missing or holds only hidden
entries, such as `.git` or a sync tool's marker. It gets `bitlog.toml`, the
default projects, `templates/note.md` and `README.md`. `/.bitlog/` is added
to `.gitignore` and `/.bitlog` to `.stignore`, extending files that already
exist. The other files and folders are created when first needed.

## Vault configuration: `bitlog.toml`

```toml
format = 1
name = "My BitLog"

[week]
first_day = "mon"
workdays = ["mon", "tue", "wed", "thu"]
target_hours = 32.0              # display only

[grid]
slot_minutes = 15
day_start = 07:00:00             # visible range
day_end = 19:00:00

[locations]
remote = "Remote"
office = "Office"
hybrid = "Hybrid"                # partly remote, partly in the office

[defaults]
location = "remote"
note_template = "templates/note.md"

[projects]
order = ["project-a", "meetings", "filler"]
```

Only `format` is required. Missing fields take these defaults:

| Field | Default | Rule |
| --- | --- | --- |
| `name` | `"BitLog"` | |
| `week.first_day` | `"mon"` | weekday, short (`mon`) or long (`monday`) |
| `week.workdays` | `["mon", "tue", "wed", "thu", "fri"]` | weekdays, at least one; `target_hours` are shared evenly among them, a day of another `kind` than `work` takes its share off |
| `week.target_hours` | `40.0` | not negative |
| `grid.slot_minutes` | `15` | divides 60 |
| `grid.day_start` | `07:00:00` | TOML local time, before `day_end` |
| `grid.day_end` | `19:00:00` | TOML local time |
| `locations` | none | keys are location keys, values display names |
| `defaults.location` | none | a key from `[locations]`; the location of new days |
| `defaults.note_template` | none | path of a file in the vault, relative to it, without `..` |
| `projects.order` | none | project slugs, the order projects are listed in |

Projects missing from `projects.order` are listed after the others, by name.
Slugs of projects the vault lacks are ignored. When a project is moved, the
app writes the order of all projects.

A new vault is created with the locations `remote`, `office` and `hybrid`
as in the example; `hybrid` is meant for days spent partly in each place.

## Day file: `YYYY-MM-DD.md`

A day is **one Markdown file** with a YAML front matter. The front matter holds
all structured data. It is followed by the date heading, the optional day note
and one section per block. The outline is fixed: `#` is the day, `##` are the
blocks, and below them there is only text without headings.

Day files live in `daily/YYYY/MM/`, the folder of their year and month. Only
files named exactly `YYYY-MM-DD.md` there are day files. Everything else, such
as sync conflict copies, is ignored when reading days.

````markdown
---
format: 1
date: "2026-09-23"
kind: "work"
location: "remote"
tags: ["refactoring"]
energy: 4
blocks:
  - { id: "k7f3", start: "08:30", end: "10:00", project: "project-a" }
  - { id: "m2q8", start: "10:00", end: "10:15", project: "meetings" }
  - { id: "p4w1", start: "10:15", end: "12:00", project: "project-b" }
  - { id: "b8n2", start: "12:00", end: "12:30", project: "pause" }
---

# 2026-09-23

Quiet day, finally time for the refactoring.

## Rebuilt auth middleware {#k7f3}

Token refresh now goes through the **interceptor**.

**Open:**

- [ ] `AuthGuard` still needs to be adapted

```rust
let token = refresh(&session).await?;
```

## Daily {#m2q8}

## {#p4w1}

## Lunch {#b8n2}
````

### Front matter

- YAML front matter between two `---` lines at the start of the file, read as
  **YAML 1.2**.
- Blocks are a list of flow mappings, **one block per line**, sorted by start
  time.
- Times are strings `"HH:MM"`, the date is a string `"YYYY-MM-DD"`, both in the
  user's local time zone. BitLog validates both itself.
- **Canonical form:** BitLog rewrites the front matter completely on every
  save, in a fixed field order, with **all strings in double quotes**. This
  avoids the well-known YAML pitfalls (`no` as a boolean, `10:30` as a number).
  The order is `format`, `date`, `kind`, `location`, `tags`, `energy`,
  `blocks`, as in the example above. `kind` is always written; the other
  optional fields only when they have a value.
- **Tolerant reading:** Unquoted values, multi-line lists and other valid YAML
  notations are accepted, for example after Obsidian has reformatted the front
  matter. Unknown fields are preserved and written back at the end of the front
  matter, in their order, with their values in JSON notation (valid YAML 1.2,
  strings in double quotes). Comments in the front matter are lost on writing.

| Field | Type | Required |
| --- | --- | --- |
| `format` | integer | yes |
| `date` | `"YYYY-MM-DD"`, matching the file name | yes |
| `kind` | string | no, defaults to `work` |
| `location` | key from `[locations]` | no |
| `tags` | list | no |
| `energy` | 1–5 | no |
| `blocks[].id` | 4 characters `[a-z0-9]`, unique within the day | yes |
| `blocks[].start`, `.end` | `"HH:MM"`, not equal | yes |
| `blocks[].project` | project slug | yes |

### Date heading

- Directly after the front matter comes `# YYYY-MM-DD`, matching `date`. It is
  owned by BitLog and deliberately language-independent; the weekday is only
  shown in the app.
- It is optional when reading. If it is missing, BitLog adds it on the next
  write.

### Day note

- Everything between the date heading and the first block heading. Optional,
  restricted to block Markdown (see below).

### Block Markdown

Block texts and the day note use a restricted subset of Markdown, because the
outline of the document belongs to BitLog:

- **Allowed:** paragraphs, lists and task lists, code blocks, inline code,
  bold, italic, strikethrough, links, images (see "Images"), block quotes,
  tables, tags (`#tag`, without a space after the hash).
- **Not intended:** headings of any level, both `# Text` and setext headings
  (`===` or `---` below a line of text).
- **In the app:** The editor offers no headings. Typed heading syntax is shown
  as text and escaped on save (`\# Text`, `\---`), so that it appears as text in
  every Markdown viewer. The backslash is dimmed in the editor.
- **Outside the app:** Hand-written headings in block texts are tolerated and
  belong to the text of the block. BitLog does not change them unasked;
  `bitlog doctor` reports them and offers to escape them.
- **Wiki links** to notes work as in project notes (see there), but always
  name the project, as in `[[project-a/name]]`: a day file belongs to no
  project, not even in a block, so `[[name]]` points nowhere there.
  `bitlog doctor` reports such links in block texts, which used to point to
  a note of the block's project, and offers to name that project in them.
- Project notes are not affected; they may use full Markdown.

### Block sections

- Every block has a level 2 heading: `## Title {#id}`. The heading is the title
  of the block, a single line. Without a title it reads `## {#id}`.
- **Only** a level 2 heading that ends in `{#id}` and whose ID is listed in the
  front matter **counts as a block boundary.** It has to start at the beginning
  of a line, as `## `. The Markdown around it does not matter: it counts even
  inside a code block, so that a code block left open cannot hide the blocks
  after it. All other headings belong to the text of the preceding block (see
  block Markdown).
- If a block has more than one heading, the first one counts. The others are
  read as text, with a warning. BitLog escapes them on writing
  (`\## Title {#id}`), also in code blocks, so that they stay text when
  sections are reordered. `bitlog doctor` reports escaped headings of blocks:
  the text below them may belong to their block.
- Blank lines around the day note and around a block text are layout, not
  text; BitLog writes exactly one blank line there.
- The text below a block heading is the **block text**. It belongs to the user
  and is written back character for character.
- BitLog writes the sections in order of their start times.
- If a block in the front matter has no heading, BitLog adds it on the next
  write.
- A heading with an ID marker whose ID is not in the front matter is an
  ordinary hand-written heading: it stays where it is and belongs to the text
  of the preceding block, or to the day note if no block precedes it. It
  produces a warning, and `bitlog doctor` reports it and offers to escape it.
- When a block that has text is deleted, BitLog asks whether to discard the
  text or move it to the day note. Moved text is appended to the day note
  below a bold line naming the block, such as `**09:00–10:30 project-a: Title**`.

### Block rules

- Blocks do not overlap. BitLog prevents it within a day; when reading,
  overlaps are flagged, not discarded. A block past midnight is not checked
  against the blocks of the next day.
- Blocks get their project from the vault's projects and the day its location
  from `[locations]` when set in BitLog. Unknown values read from a file are
  kept.
- Blocks do not have to be contiguous. The point is a rough assignment.
- An `end` earlier than `start` means the next day. The block counts towards
  the day it starts on.
- **Breaks are blocks** of a project with the category `break`. Working time is
  the sum of all blocks except breaks; time without a block does not count.
  Loose work such as e-mails or chats goes into blocks of `filler`. Blocks of
  projects that do not exist count as work.

### Rationale

- One file per day instead of one file per block stays manageable over years.
- Directly readable in any Markdown viewer, on code hosting sites and by
  language models.
- YAML front matter is the standard in the Markdown ecosystem: Obsidian shows
  it as properties, many viewers render it as a table, Pandoc and many editors
  understand it.
- Known drawbacks, accepted deliberately: no real time types (hence strings and
  own validation), no format-preserving YAML editing in Rust (hence canonical
  rewriting; comments in the front matter are lost).
- Standalone files (`bitlog.toml`, `project.toml`, `tasks.toml`) stay TOML,
  because they are also edited and commented by hand, and TOML can be edited
  while preserving formatting and comments.

## Project: `project.toml`

```toml
format = 1
name = "Project A"
color = "#3584e4"
status = "active"                # active | paused | archived
category = "work"                # free text; `break` is the only special category
created = 2026-03-01
```

The project's slug is the name of its folder. A folder below `projects/` is a
project only if its name is a valid slug and it contains a `project.toml`;
anything else there is ignored.

Only `format` is required. Missing fields take these defaults:

| Field | Default | Rule |
| --- | --- | --- |
| `name` | the slug | |
| `color` | `"#3584e4"` | `#` and six hex digits |
| `status` | `"active"` | `active`, `paused` or `archived` |
| `category` | `"work"` | free text; blocks of `break` projects are breaks |
| `created` | none | TOML local date |

BitLog writes all fields except a missing `created`. Comments, formatting,
unchanged values and unknown fields are kept as they are. The order of
projects is set in `bitlog.toml`.

The path to a project's local Git repository differs on every machine, so it
is stored per device in `.bitlog/device.toml`:

```toml
[repos]
project-a = "/home/me/code/project-a"
```

Each entry maps a project slug to the absolute path of a folder with a Git
repository. A missing file or table means no repositories. Entries of
unknown projects are ignored and, like comments and other tables, kept when
BitLog writes the file. BitLog only sets paths of folders that hold
`.git`.

**Renaming** a project changes its slug, which must not be taken. BitLog
renames its folder, sets the new slug in all its blocks, in `projects.order`
and in `.bitlog/device.toml`, and changes the targets of wiki links to its
notes in all notes, day notes and block texts.

**Default projects of a new vault:**

| Slug | Category | Counts as working time |
| --- | --- | --- |
| `pause` | `break` | no |
| `meetings` | `overhead` | yes |
| `filler` | `overhead` | yes |

`filler` stands for loose time: research, e-mails, longer chats, short
digressions.

## Tasks: `tasks.toml`

A **global task list** at the vault root, unrelated to projects, blocks or
days. It is meant for small things like "get back to XYZ about the handover" or
"take the keyboard to the office". Nothing is carried over from day to day; the
list is simply always there.

```toml
format = 1

[[task]]
id = "t9x2"
title = "Get back to XYZ about the handover"
status = "open"                  # open | done | dropped
created = 2026-09-22
due = 2026-09-30

[[task]]
id = "r3m7"
title = "Take the keyboard to the office"
status = "done"
created = 2026-09-22
done = 2026-09-23                # when it was done or dropped
```

Only `format` is required, and `id` and `title` for each task. Missing fields
take these defaults:

| Field | Default | Rule |
| --- | --- | --- |
| `id` | – | a task ID, unique within the file |
| `title` | – | one line of text |
| `status` | `"open"` | `open`, `done` or `dropped` |
| `created` | none | TOML local date |
| `due` | none | TOML local date |
| `done` | none | TOML local date; the day the task was done or dropped |

- A missing `tasks.toml` is an empty list. BitLog creates it with the first
  task.
- BitLog writes open tasks first, then done and dropped ones, each in the
  order chosen by the user. New tasks go to the end of the open tasks. A task
  that is done or dropped gets today as `done` and moves to the top of the
  finished tasks; opened again, it loses `done` and moves to the end of the
  open tasks.
- Comments, formatting, unchanged values and unknown fields are kept as they
  are. Tasks may also be written by hand as an array of inline tables;
  BitLog writes them as `[[task]]` tables.
- **Archiving** happens only when the user asks for it. It moves all done and
  dropped tasks to `tasks-archive-YYYY.toml`, by the year of `done`, else of
  `created`, else of the current date. An archive has the same format as
  `tasks.toml` and is extended at the end; a task whose ID is taken in the
  archive gets a new one there.
- **Project tasks** are reserved for a later format extension. They will live
  in separate files per project and will not change the global list.

## Project notes

- Live in `projects/<slug>/notes/` and belong to the project only through this
  folder.
- Plain Markdown; front matter is allowed but not evaluated.
- BitLog only reads wiki links from them. Tags are defined below, but not
  evaluated yet.
- Full Markdown, including headings.
- Checkboxes are text, not managed tasks.
- Only `.md` files directly in `notes/` are notes; subfolders are ignored.

**Wiki links** point to a note by its project and name:

| Link | Points to |
| --- | --- |
| `[[project-a/deployment]]` | `projects/project-a/notes/deployment.md` |
| `[[deployment]]` | `deployment.md` in the notes of the same project |
| `[[project-a/deployment#Steps]]` | the same note; the heading is not checked |
| `[[project-a/deployment\|how we deploy]]` | the same note, shown as "how we deploy" |

Links in code are text. A target that is no valid note path, such as
`[[a/b/c]]`, points nowhere.

**Tags** are words starting with `#`, followed by letters, digits, `-`, `_`
or `/`, such as `#release` or `#area/backend`. A `#` followed by digits only,
like `#123`, is no tag, so issue numbers stay what they are. Code, front
matter and the `#` inside a word or link are no tags.

**The template** named by `defaults.note_template` is used for new notes. It
may contain these placeholders, which are replaced once when a note is
created; any other text in `{{…}}` stays as it is:

| Placeholder | Replaced by |
| --- | --- |
| `{{title}}` | the note's name |
| `{{project}}` | the project's name |
| `{{date}}` | today as `YYYY-MM-DD` |
| `{{date:FORMAT}}` | today in a strftime format such as `%d.%m.%Y` |

Without a template, or if the file is missing, a new note is empty.

**Renaming** a note changes only its file name. BitLog offers to change the
wiki links to it in all notes, day notes and block texts as well; it then
replaces only the link targets and keeps the rest of each link.

## Project assets

- Live in `projects/<slug>/assets/` and its subfolders, and belong to the
  project only through this folder. They may be files of any kind.
- BitLog never reads them; it lists them and leaves opening them to other
  apps.
- Hidden files and folders, whose names start with `.`, are no assets, nor
  is anything in them. Sync conflict copies are assets of their own.
- An asset added under a name that is taken gets a number before its
  extension, as in `offer (2).pdf`.
- BitLog keeps no links to assets. Renaming or removing one changes
  nothing else, not even an image link to it (see "Images").

## Images

- Texts show images with Markdown image links whose target is a path
  relative to the file of the text, with `/` between folders:
  `![Login form](../../../images/login-form.png)`. Day files and project
  notes are both three folders deep, so this path is the same in all texts.
- Images live in `images/`, which may have subfolders. Any other image of
  the vault can be shown as well, such as a project asset:
  `![](../assets/scans/rack.jpg)` in a project note.
- A target that leaves the vault, an absolute path and a web address show
  no image. Spaces in a target are written `%20`, or the whole target is
  put in angle brackets: `![](<../../../images/login form.png>)`.
- BitLog adds images to `images/` itself, not to its subfolders. An added
  file keeps its name, with a number before its extension if the name is
  taken, as in `login (2).png`. A file that lies in the vault already is
  linked where it is. A pasted image without a name is named after
  the time it was pasted, as in `2026-10-06 12-34-56.png`. An image whose
  content is in `images/` or its subfolders already is linked rather than
  added again, the first by path if several have it.
- BitLog writes image links without alternative text, the target in angle
  brackets if it has spaces or parentheses, each on a line of its own.
- `bitlog doctor` reports image links to files of the vault that are
  missing, and links to images outside the vault, such as absolute paths:
  they show nothing and lead elsewhere on other devices. It also reports
  images in `images/` that no text shows, unless a text cannot be read, and
  images there with the same content.

## Exports: `exports/`

BitLog writes exports on request and overwrites an earlier export of the
same name. Nothing reads them back.

| File | Content |
| --- | --- |
| `blocks-FIRST-LAST.csv`, `blocks.csv` | the blocks from `FIRST` to `LAST` (`YYYY-MM-DD`), or all of them |
| `remote-days.csv` | the work days with the location `remote` or `hybrid`, per year |
| `week-FIRST.md` | a report of the week starting on `FIRST` |

The CSV files follow RFC 4180 (UTF-8, comma, CRLF, a header line). The
blocks have the columns `date`, `start`, `end`, `minutes`, `project`,
`project_name`, `category`, `title` and `text`; an `end` before `start` is on
the next day, as in the day files. The week report lists the time per project
without breaks, then each day with its details, day note and blocks with
their texts.

## Writing rules

| File or part | Written by | When |
| --- | --- | --- |
| `bitlog.toml`, `tasks.toml`, `project.toml` | BitLog | any time, preserving formatting and comments |
| `tasks-archive-YYYY.toml` | BitLog | when archiving, preserving formatting and comments |
| Day file: front matter, date heading and block headings | BitLog | any time |
| Day file: day note and block texts | user | always; BitLog keeps them unchanged, except for wiki links to a renamed note, if the user agrees, or to the notes of a renamed project |
| Project notes | user | always; BitLog only when creating them from the template, in wiki links to a renamed note, if the user agrees, and in wiki links to the notes of a renamed project |
| `exports/*` | BitLog | may be overwritten completely |
| `.bitlog/*` | BitLog | local |

Every write:

1. Remember a hash of the file when loading it.
2. Check the hash before saving. A difference means an external change: reload
   the file and apply the own change again. If the same block text or note was
   changed on both sides, offer a comparison.
3. Write to a temporary file in the same folder, named
   `.<file name>.<random>.tmp`, then rename it atomically. A file whose
   content does not change is not written at all.
4. An unchanged day file in canonical form must be **byte-identical** after
   reading and writing.

## Sync conflicts

- Recognised are conflict copies of the files above, named
  `<name>.sync-conflict-<date>-<time>-<device><extension>` (Syncthing) or
  `<name> (conflicted copy…)<extension>` (Nextcloud), in the folder of their
  original. Copies are never read as days, notes or tasks.
- A copy has no common ancestor with its original, so BitLog cannot tell
  which side changed something. It takes what both sides agree on and what
  only one side has; an empty text or a missing field gives way to the other
  side. Everything else is a contradiction that the user decides. A block or
  task removed on one side therefore comes back.
- Day files: blocks are merged by their IDs, a block only the copy has only
  if it overlaps no block of the original. Front matter fields, block times,
  projects, titles and texts and the day note are compared one by one.
- `tasks.toml`: tasks are merged by their IDs, field by field. A task only
  one side has whose ID is in a task archive was archived on the other side
  and stays archived.
- Project notes, `project.toml` and `bitlog.toml`: never merged; the user
  picks a version.
- A copy that is the same as its original, or whose original is missing, is
  merged without asking. After merging, the copy is removed.
- Git conflict markers (lines starting with `<<<<<<<` and `>>>>>>>`) are
  only reported: in project notes by `bitlog doctor`, in all other files
  as the reason they cannot be read. They are resolved with Git.
- `bitlog doctor` lists the conflict copies with their contradictions, and
  `bitlog doctor --fix` merges those without any.
- The app merges copies without contradictions as soon as it sees them. For
  the others, the user chooses the original's or the copy's side of each
  contradiction, a text after comparing both versions. Taking a block that
  only the copy has removes the blocks of the original it overlaps.

## Versioning

- Every TOML file and every day front matter carries `format`. BitLog reads
  all known versions and writes the current one.
- New optional fields do not increase the version; only renames or changes in
  meaning do, each with a migration.
