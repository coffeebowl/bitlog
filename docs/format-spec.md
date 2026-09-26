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

## Folder structure

```
my-vault/
  README.md                          # explains the format to humans and LLMs
  knotbook.toml
  tasks.toml                         # global task list
  daily/2026/09/2026-09-23.md        # one day: front matter, day note, blocks with text
  projects/project-a/project.toml
  projects/project-a/notes/auth-middleware.md
  templates/note.md
  exports/                           # generated, may be overwritten
  .knotbook/                         # local, never sync
    index.sqlite
    device.toml                      # per device, e.g. repository paths
```

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
homeoffice = "Home office"
office = "Office"
travel = "Travelling"

[defaults]
location = "homeoffice"
note_template = "templates/note.md"
```

## Day file: `YYYY-MM-DD.md`

A day is **one Markdown file** with a YAML front matter. The front matter holds
all structured data. It is followed by the date heading, the optional day note
and one section per block. The outline is fixed: `#` is the day, `##` are the
blocks, and below them there is only text without headings.

````markdown
---
format: 1
date: "2026-09-23"
kind: "work"
location: "homeoffice"
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
- **Tolerant reading:** Unquoted values, multi-line lists and other valid YAML
  notations are accepted, for example after Obsidian has reformatted the front
  matter. Unknown fields are preserved and written back at the end of the front
  matter. Comments in the front matter are lost on writing.

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
| `blocks[].start`, `.end` | `"HH:MM"` | yes |
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
- Project notes are not affected; they may use full Markdown.

### Block sections

- Every block has a level 2 heading: `## Title {#id}`. The heading is the title
  of the block. Without a title it reads `## {#id}`.
- **Only** a level 2 heading that ends in `{#id}` and whose ID is listed in the
  front matter **counts as a block boundary.** All other headings belong to the
  text of the preceding block (see block Markdown). Headings inside code blocks
  are ignored.
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
  text or move it to the day note.

### Block rules

- Blocks do not overlap. Knotbook prevents it; when reading, overlaps are
  flagged, not discarded.
- Blocks do not have to be contiguous. The point is a rough assignment.
- An `end` earlier than `start` means the next day. The block counts towards
  the day it starts on.
- **Breaks are blocks** of a project with the category `break`. Working time is
  `work.end − work.start − sum(break blocks)`. If the start or end of work is
  missing, working time is the sum of all blocks except breaks.

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

[[task]]
id = "r3m7"
title = "Take the keyboard to the office"
status = "done"
created = 2026-09-22
done = 2026-09-23
```

- Task IDs: 4 characters from `[a-z0-9]`, unique within the file.
- Knotbook writes open tasks first, in the order chosen by the user.
- Above a threshold, finished tasks move to `tasks-archive-YYYY.toml`.
- **Project tasks** are reserved for a later format extension. They will live
  in separate files per project and will not change the global list.

## Project notes

- Live in `projects/<slug>/notes/` and belong to the project only through this
  folder.
- Plain Markdown; front matter is allowed but not evaluated.
- Knotbook only reads wiki links (`[[project-a/deployment]]`) and tags (`#tag`)
  from them.
- Full Markdown, including headings.
- Checkboxes are text, not managed tasks.
- The template `templates/note.md` may contain the placeholders `{{title}}`,
  `{{date:FORMAT}}` and `{{project}}`, which are replaced once when a note is
  created.

## Writing rules

| File or part | Written by | When |
| --- | --- | --- |
| `knotbook.toml`, `tasks.toml`, `project.toml` | Knotbook | any time, preserving formatting and comments |
| Day file: front matter, date heading and block headings | Knotbook | any time |
| Day file: day note and block texts | user | always; Knotbook keeps them unchanged |
| Project notes | user | always; Knotbook only when creating them from the template |
| `exports/*` | Knotbook | may be overwritten completely |
| `.knotbook/*` | Knotbook | local |

Every write:

1. Remember a hash of the file when loading it.
2. Check the hash before saving. A difference means an external change: reload
   the file and apply the own change again. If the same block text or note was
   changed on both sides, offer a comparison.
3. Write to a temporary file in the same folder, then rename it atomically.
4. An unchanged day file in canonical form must be **byte-identical** after
   reading and writing.

## Sync conflicts

- Recognised are `*.sync-conflict-*` (Syncthing), `* (conflicted copy)*`
  (Nextcloud) and Git conflict markers.
- Day files: the front matter is merged by block IDs; blocks are merged
  automatically if they do not overlap. Block texts are merged per section:
  automatically if only one side changed it, otherwise in a comparison view.
- `tasks.toml`: field by field via IDs. On contradictions Knotbook asks.
- Project notes: never automatically, always in a comparison view.

## Versioning

- Every TOML file and every day front matter carries `format`. Knotbook reads
  all known versions and writes the current one.
- New optional fields do not increase the version; only renames or changes in
  meaning do, each with a migration.
- This document and JSON schemas for the TOML files (for Taplo) and the day
  front matter are part of the Knotbook repository.
