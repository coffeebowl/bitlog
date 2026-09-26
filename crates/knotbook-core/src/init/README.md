# Knotbook vault

This folder is a Knotbook vault: a daily dev log kept as plain text files.
Every file can be read and corrected by hand. The app Knotbook is only one way
to work with them.

## Files

- `knotbook.toml` – settings of the vault: week, time grid and locations.
- `daily/YYYY/MM/YYYY-MM-DD.md` – one Markdown file per day.
- `projects/<slug>/project.toml` – one folder per project, with Markdown notes
  in `notes/`.
- `tasks.toml` – a global list of small tasks, once there are any.
- `templates/note.md` – the template for new project notes.
- `.knotbook/` – local data of this device, such as the search index. It can
  be deleted at any time and is never synchronised.

## Days

A day file starts with YAML front matter holding the day's details and its
**blocks**: time spans assigned to a project, identified by a short ID.

```markdown
---
format: 1
date: "2026-09-23"
kind: "work"
location: "remote"
work: { start: "08:30", end: "16:45" }
blocks:
  - { id: "k7f3", start: "08:30", end: "10:00", project: "project-a" }
  - { id: "b8n2", start: "12:00", end: "12:30", project: "pause" }
---

# 2026-09-23

A note about the whole day.

## Rebuilt auth middleware {#k7f3}

Free text about this block.

## Lunch {#b8n2}
```

- Below the date heading comes the day note, then one section per block. A
  section's heading is the block's title and ends with its ID, `{#k7f3}`.
- Times are local. An end before the start means the block runs past
  midnight; it belongs to the day it starts on.
- Blocks of projects with the category `break` are breaks. Working time is
  `work.end − work.start` minus the breaks, or the sum of all other blocks if
  the start or end of work is missing.
- The text below a heading belongs to the user and is kept as written. Block
  texts and the day note contain no headings of their own.

## Project notes

Project notes are plain Markdown files, headings included. A wiki link
`[[project-a/deployment]]` points to `projects/project-a/notes/deployment.md`,
`[[deployment]]` to a note of the same project. Words like `#release` are
tags.

The full format is described in `docs/format-spec.md` in the Knotbook
repository.
