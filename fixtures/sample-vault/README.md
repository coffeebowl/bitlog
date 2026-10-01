# Sample BitLog vault

This folder is a BitLog vault: a daily dev log kept as plain text files.
It is a test fixture and contains some deliberately unusual files.

- `bitlog.toml` – settings of the vault.
- `daily/YYYY/MM/YYYY-MM-DD.md` – one Markdown file per day. The YAML front
  matter lists the day's details and its blocks (time spans assigned to a
  project). Below it, every block has a section `## Title {#id}` with free
  text.
- `projects/<slug>/project.toml` – one folder per project, with notes in
  `notes/`.
- `tasks.toml` – a global list of small tasks.
- `templates/` – templates for new notes.

The full format is described in `docs/format-spec.md` in the BitLog
repository.
