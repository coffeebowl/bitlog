# BitLog vault

This folder is a BitLog vault: a daily dev log kept as plain text files.
Every file can be read and corrected by hand. The app BitLog is only one way
to work with them.

## Files

- `bitlog.toml` – settings of the vault: week, time grid and locations.
- `days/YYYY/MM/YYYY-MM-DD.md` – one Markdown file per day.
- `projects/<slug>/project.toml` – one folder per project, with Markdown notes
  in `notes/` and any other files, such as PDFs or images, in `assets/`.
- `images/` – images shown in day notes, block texts and project notes,
  linked like `![Login form](../../../images/login-form.png)`.
- `tasks.toml` – a global list of small tasks, once there are any.
- `contacts/<slug>.toml` – one file per person, with phone numbers, e-mail
  addresses and their roles in projects; `contacts/<slug>.jpg` is their
  picture.
- `templates/note.md` – the template for new project notes.
- `.bitlog/` – local data of this device, such as the search index. It can
  be deleted at any time and is never synchronised.

## Days

A day file starts with YAML front matter holding the day's details and its
**blocks**: time spans assigned to a project, identified by a short ID.

```markdown
---
date: "2026-09-23"
kind: "work"
location: "remote"
blocks:
  - { id: "k7f3", start: "08:30", end: "10:00", project: "project-a" }
  - { id: "m2q8", start: "10:00", end: "10:15", project: "project-a" }
  - { id: "b8n2", start: "12:00", end: "12:30", project: "pause" }
---

# 2026-09-23

A note about the whole day.

## Rebuilt auth middleware {#k7f3}

Free text about this block.

## Daily {#m2q8}

## Lunch {#b8n2}
```

- Below the date heading comes the day note, then one section per block. A
  section's heading is the block's title and ends with its ID, `{#k7f3}`.
- Times are local. An end before the start means the block runs past
  midnight; it belongs to the day it starts on.
- Every project has a category in its `project.toml`. Working time is the
  sum of the blocks of `work` and `duty` projects; blocks of `private` and
  `break` projects and time without a block do not count.
- The text below a heading belongs to the user and is kept as written. Block
  texts and the day note contain no headings of their own.

## Project notes

Project notes are plain Markdown files, headings included. A wiki link
`[[project-a/deployment]]` points to `projects/project-a/notes/deployment.md`,
`[[deployment]]` to a note of the same project.

The full format is described in `docs/format-spec.md` in the BitLog
repository.
