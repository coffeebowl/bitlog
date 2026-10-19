# Architecture

How the code of BitLog is organised, to find your way around. The vault
format is described in [format-spec.md](format-spec.md), building in
[building.md](building.md).

## Principles

- **Files are the only source of truth.** A vault is a folder of plain
  text files. The app owns their structure, the user owns the text: what
  the user writes is kept as it is. Files are read tolerantly and written
  strictly.
- **The index is a cache.** `.bitlog/index.sqlite` is never synced and can
  be deleted at any time; it is rebuilt from the files.
- **Changes never overwrite changes made elsewhere.** Before a file is
  written, its hash is compared with the one it was read with. If it was
  changed meanwhile, for example by a sync tool, it is read again and the
  change is applied to the new content.
- **No server, no network.** Syncing is left to Git, Syncthing or any
  other tool.

## Crates

```
bitlog-core     data model, reading and writing a vault
  ↑
bitlog-index    SQLite index: search, sums, backlinks, exports
  ↑
bitlog-cli (`bitlog`)   ·   app (`bitlog-gtk`)
```

- **`crates/bitlog-core`** knows the vault: days, projects, notes, assets,
  contacts, tasks and settings. `vault.rs` is the entry point. It also finds
  and merges sync conflict copies (`conflict.rs`), checks a vault for
  problems (`check.rs`), watches it for changes (`watch.rs`), reads Git logs
  and holds the Markdown rules of the editor (`markdown/`).
- **`crates/bitlog-index`** fills the index from the vault and answers
  searches, sums and backlinks. Its schema grows only by new files in
  `migrations/`.
- **`crates/bitlog-cli`** is the command line tool, see
  [command-line.md](command-line.md). Its output is English only.
- **`app`** is the GNOME app, built with GTK 4, libadwaita and Meson.

## The app

| Folder or file | What it holds |
|---|---|
| `src/main.rs` | Starting the app |
| `src/window/` | The main window: sidebar, navigation, opening and watching the vault |
| `src/day_view/`, `src/timeline/` | The day page and its timeline of blocks |
| `src/calendar_view.rs`, `src/reports_page.rs` | Calendar and reports |
| `src/tasks_page.rs`, `src/task_list_view.rs` | Tasks; the list also shows on the day page |
| `src/notes_page.rs`, `src/note_view.rs` | All notes, and one note |
| `src/projects_page.rs`, `src/project_view/`, `src/git_page.rs` | Projects, one project, and its Git log |
| `src/contacts_page/`, `src/contact_view.rs` | Contacts, and one contact |
| `src/markdown_view/` | The Markdown editor used for all texts |
| `src/dialogs/` | All dialogs, including help and preferences |
| `src/widgets/` | Small widgets pages are built from, such as charts and cards, and shared widget helpers |
| other files in `src/` | Shared helpers: formats, colours, drawing, the index, trash, … |
| `ui/` | Blueprint templates, named like the Rust file they belong to |

Outside `app/`, `data/` holds the desktop file, metainfo, settings schema,
icons and the style sheet, and `po/` the translations.

A few patterns hold throughout the app:

- Every page gets the opened vault with `set_vault`. A change is made on a
  copy of the vault, saved, and then handed to all pages again.
- Slow work, such as index queries, runs on a worker thread
  (`gio::spawn_blocking`) so that the window never hangs.
- Texts are saved a second after the last keystroke, and when they lose
  focus or the window closes.

## Tests

The crates are tested with `cargo test --workspace`. The tests sit at the
end of each file, in `mod tests`. Most of them work on a copy of
`fixtures/sample-vault/`, so the fixtures themselves are never changed. The
app is checked by hand.

## Other folders

- `build-aux/`: the Flatpak manifest and the script that bundles a release.
- `fixtures/sample-vault/`: a small vault for tests and for trying the app.
- `docs/`: this and the other documents.
