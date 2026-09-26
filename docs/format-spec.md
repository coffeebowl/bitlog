# Knotbook file format

**Status:** Draft of format version 1. This document describes the target
format. Knotbook implements it step by step, so parts of it may not be
supported by the current code yet.

A Knotbook vault is an ordinary folder of plain text files. Every file can be
read and corrected by hand, in any text editor, without Knotbook. The files are
the only source of truth; the local index and caches can be rebuilt from them
at any time.

## Terms

| Term | Meaning | Stored in |
| --- | --- | --- |
| Vault | The folder holding all data | `knotbook.toml` at its root |
| Day | One calendar day of work | `daily/YYYY/MM/YYYY-MM-DD.md` |
| Day note | Free text about the whole day | Day file, below the date heading |
| Block | A time span of a day assigned to one project | Day file, front matter and one section |
| Block text | Free text about one block | Day file, below the block heading |
| Project | Something time is spent on | `projects/<slug>/project.toml` |
| Project note | A Markdown note belonging to a project | `projects/<slug>/notes/*.md` |
| Task | An item on the global task list | `tasks.toml` |

A block refers to its project by slug. A project note belongs to its project
only through the folder it lives in. There are no references between blocks
and project notes.

## Identifiers

| Identifier | Rule | Example |
| --- | --- | --- |
| Project slug, location key | lowercase letters `a-z`, digits and single hyphens, not at the start or end | `project-a` |
| Block ID, task ID | exactly 4 characters from `a-z` and `0-9` | `k7f3` |
| Note name | the file name without `.md`; not empty, no `/` or `\`, not starting with `.`, not a sync conflict copy | `Auth middleware` |

A project note is identified by its path relative to the vault,
`projects/<slug>/notes/<name>.md`, always written with `/`.

## Folder structure

```
my-vault/
  README.md                          # explains the format to humans and LLMs
  knotbook.toml
  tasks.toml                         # global task list
  tasks-archive-2026.toml            # tasks finished in 2026, once archived
  daily/2026/09/2026-09-23.md        # one day: front matter, day note, blocks with text
  projects/project-a/project.toml
  projects/project-a/notes/auth-middleware.md
  templates/note.md
  exports/                           # generated, may be overwritten
  .knotbook/                         # local, never sync
    index.sqlite
    device.toml                      # per device, e.g. repository paths
```

A new vault is created in a folder that is missing or holds only hidden
entries, such as `.git` or a sync tool's marker. It gets `knotbook.toml`, the
default projects, `templates/note.md` and `README.md`. `/.knotbook/` is added
to `.gitignore` and `/.knotbook` to `.stignore`, extending files that already
exist. The other files and folders are created when first needed.

## Vault configuration: `knotbook.toml`

```toml
format = 1
name = "My Knotbook"

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
```

Only `format` is required. Missing fields take these defaults:

| Field | Default | Rule |
| --- | --- | --- |
| `name` | `"Knotbook"` | |
| `week.first_day` | `"mon"` | weekday, short (`mon`) or long (`monday`) |
| `week.workdays` | `["mon", "tue", "wed", "thu", "fri"]` | weekdays |
| `week.target_hours` | `40.0` | not negative |
| `grid.slot_minutes` | `15` | divides 60 |
| `grid.day_start` | `07:00:00` | TOML local time, before `day_end` |
| `grid.day_end` | `19:00:00` | TOML local time |
| `locations` | none | keys are location keys, values display names |
| `defaults.location` | none | a key from `[locations]`; the location of new days |
| `defaults.note_template` | none | path relative to the vault |

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
work: { start: "08:30", end: "16:45" }
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
  user's local time zone. Knotbook validates both itself.
- **Canonical form:** Knotbook rewrites the front matter completely on every
  save, in a fixed field order, with **all strings in double quotes**. This
  avoids the well-known YAML pitfalls (`no` as a boolean, `10:30` as a number).
  The order is `format`, `date`, `kind`, `location`, `tags`, `energy`, `work`,
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
| `work.start`, `work.end` | `"HH:MM"` | no |
| `blocks[].id` | 4 characters `[a-z0-9]`, unique within the day | yes |
| `blocks[].start`, `.end` | `"HH:MM"`, not equal | yes |
| `blocks[].project` | project slug | yes |

### Date heading

- Directly after the front matter comes `# YYYY-MM-DD`, matching `date`. It is
  owned by Knotbook and deliberately language-independent; the weekday is only
  shown in the app.
- It is optional when reading. If it is missing, Knotbook adds it on the next
  write.

### Day note

- Everything between the date heading and the first block heading. Optional,
  restricted to block Markdown (see below).

### Block Markdown

Block texts and the day note use a restricted subset of Markdown, because the
outline of the document belongs to Knotbook:

- **Allowed:** paragraphs, lists and task lists, code blocks, inline code,
  bold, italic, strikethrough, links, block quotes, tables, tags (`#tag`,
  without a space after the hash).
- **Not intended:** headings of any level, both `# Text` and setext headings
  (`===` or `---` below a line of text).
- **In the app:** The editor offers no headings. Typed heading syntax is shown
  as text and escaped on save (`\# Text`, `\---`), so that it appears as text in
  every Markdown viewer. The backslash is dimmed in the editor.
- **Outside the app:** Hand-written headings in block texts are tolerated and
  belong to the text of the block. Knotbook does not change them unasked;
  `knotbook doctor` reports them and offers to escape them.
- **Wiki links** to notes work as in project notes (see there). In a block
  text, `[[name]]` points to a note of the block's project; the day note
  belongs to no project, so only links like `[[project-a/name]]` point to a
  note there.
- Project notes are not affected; they may use full Markdown.

### Block sections

- Every block has a level 2 heading: `## Title {#id}`. The heading is the title
  of the block, a single line. Without a title it reads `## {#id}`.
- **Only** a level 2 heading that ends in `{#id}` and whose ID is listed in the
  front matter **counts as a block boundary.** It has to start at the beginning
  of a line, outside code blocks, quotes and lists. All other headings belong
  to the text of the preceding block (see block Markdown).
- If a block has more than one heading, the first one counts. The others are
  read as text, with a warning. Knotbook escapes them on writing
  (`\## Title {#id}`), so that they stay text when sections are reordered.
- Blank lines around the day note and around a block text are layout, not
  text; Knotbook writes exactly one blank line there.
- The text below a block heading is the **block text**. It belongs to the user
  and is written back character for character.
- Knotbook writes the sections in order of their start times.
- If a block in the front matter has no heading, Knotbook adds it on the next
  write.
- A heading with an ID marker whose ID is not in the front matter is an
  ordinary hand-written heading: it stays where it is and belongs to the text
  of the preceding block, or to the day note if no block precedes it. It
  produces a warning, and `knotbook doctor` reports it and offers to escape it.
- When a block that has text is deleted, Knotbook asks whether to discard the
  text or move it to the day note. Moved text is appended to the day note
  below a bold line naming the block, such as `**09:00–10:30 project-a: Title**`.

### Block rules

- Blocks do not overlap. Knotbook prevents it within a day; when reading,
  overlaps are flagged, not discarded. A block past midnight is not checked
  against the blocks of the next day.
- Blocks get their project from the vault's projects and the day its location
  from `[locations]` when set in Knotbook. Unknown values read from a file are
  kept.
- Blocks do not have to be contiguous. The point is a rough assignment.
- An `end` earlier than `start` means the next day. The block counts towards
  the day it starts on.
- **Breaks are blocks** of a project with the category `break`. Working time is
  `work.end − work.start − sum(break blocks)`. If the start or end of work is
  missing, working time is the sum of all blocks except breaks. A `work.end`
  before `work.start` lies on the next day. Blocks of projects that do not
  exist count as work, and working time is never negative.

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
- Standalone files (`knotbook.toml`, `project.toml`, `tasks.toml`) stay TOML,
  because they are also edited and commented by hand, and TOML can be edited
  while preserving formatting and comments.

## Project: `project.toml`

```toml
format = 1
name = "Project A"
color = "#3584e4"
status = "active"                # active | paused | archived
category = "work"                # free text; `break` is the only special category
pinned = false                   # listed first when picking a project for a block
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
| `pinned` | `false` | |
| `created` | none | TOML local date |

Knotbook writes all fields except a missing `created`. Comments, formatting,
unchanged values and unknown fields are kept as they are.

The path to a project's local Git repository differs on every machine, so it
is stored per device in `.knotbook/device.toml`:

```toml
[repos]
project-a = "/home/me/code/project-a"
```

**Default projects of a new vault:**

| Slug | Category | Counts as working time | Pinned |
| --- | --- | --- | --- |
| `pause` | `break` | no | yes |
| `meetings` | `overhead` | yes | yes |
| `filler` | `overhead` | yes | yes |

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

- A missing `tasks.toml` is an empty list. Knotbook creates it with the first
  task.
- Knotbook writes open tasks first, then done and dropped ones, each in the
  order chosen by the user. New tasks go to the end of the open tasks. A task
  that is done or dropped gets today as `done` and moves to the top of the
  finished tasks; opened again, it loses `done` and moves to the end of the
  open tasks.
- Comments, formatting, unchanged values and unknown fields are kept as they
  are. Tasks may also be written by hand as an array of inline tables;
  Knotbook writes them as `[[task]]` tables.
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
- Knotbook only reads wiki links and tags from them.
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

**Renaming** a note changes only its file name. Knotbook offers to change the
wiki links to it in all notes as well; it then replaces only the link targets
and keeps the rest of each link.

## Exports: `exports/`

Knotbook writes exports on request and overwrites an earlier export of the
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
| `knotbook.toml`, `tasks.toml`, `project.toml` | Knotbook | any time, preserving formatting and comments |
| `tasks-archive-YYYY.toml` | Knotbook | when archiving, preserving formatting and comments |
| Day file: front matter, date heading and block headings | Knotbook | any time |
| Day file: day note and block texts | user | always; Knotbook keeps them unchanged |
| Project notes | user | always; Knotbook only when creating them from the template and, if the user agrees, in wiki links to a renamed note |
| `exports/*` | Knotbook | may be overwritten completely |
| `.knotbook/*` | Knotbook | local |

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
- A copy has no common ancestor with its original, so Knotbook cannot tell
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
- Project notes, `project.toml` and `knotbook.toml`: never merged; the user
  picks a version.
- A copy that is the same as its original, or whose original is missing, is
  merged without asking. After merging, the copy is removed.
- Git conflict markers (lines starting with `<<<<<<<` and `>>>>>>>`) are
  only reported: in project notes by `knotbook doctor`, in all other files
  as the reason they cannot be read. They are resolved with Git.
- `knotbook doctor` lists the conflict copies with their contradictions, and
  `knotbook doctor --fix` merges those without any.
- The app merges copies without contradictions as soon as it sees them. For
  the others, the user chooses the original's or the copy's side of each
  contradiction, a text after comparing both versions. Taking a block that
  only the copy has removes the blocks of the original it overlaps.

## Versioning

- Every TOML file and every day front matter carries `format`. Knotbook reads
  all known versions and writes the current one.
- New optional fields do not increase the version; only renames or changes in
  meaning do, each with a migration.
- This document and JSON schemas for the TOML files (for Taplo) and the day
  front matter are part of the Knotbook repository.
