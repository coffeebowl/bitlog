<div align="center">
  <img src="data/icons/hicolor/scalable/apps/dev.bitlog.BitLog.svg" width="128" height="128" alt="">
  <h1>BitLog</h1>
  <p>A daily dev log for developers, made for Linux and GNOME.</p>
</div>

BitLog splits the working day into 15-minute blocks and assigns them to
projects – roughly and in hindsight, without a stopwatch. Blocks can carry
Markdown notes, projects keep a small set of notes and files of their own.
All data lives in plain text files in an ordinary folder, the vault, which
you can sync with Syncthing, Nextcloud, Git or anything else. There is no
server.
The file format is described in [docs/format-spec.md](docs/format-spec.md).

BitLog is at an early stage of development.

## Installing

Every [release](https://github.com/coffeebowl/bitlog/releases) comes with
a Flatpak bundle, `bitlog-<version>.flatpak`. Download it and install it,
either with a double click in GNOME Software or KDE Discover, or on the
command line:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user ./bitlog-<version>.flatpak
flatpak run dev.bitlog.BitLog
```

Flatpak fetches the GNOME runtime from Flathub along the way. The bundle
does not update itself: to update, install the bundle of the new release
with `flatpak install --user --reinstall`.

## Building

BitLog is written in Rust (stable). The app needs GTK 4.22, libadwaita 1.9
and GtkSourceView 5.20 or newer, and is built with Meson and Blueprint. On
Fedora:

```sh
sudo dnf install gtk4-devel libadwaita-devel gtksourceview5-devel meson ninja-build \
  blueprint-compiler gettext desktop-file-utils appstream
```

Cargo alone is enough for checks and tests:

```sh
cargo test --workspace
```

The app gets its paths from Meson at compile time. To build it, install it
into `_install/` and start it:

```sh
meson setup _build --prefix="$PWD/_install"
meson install -C _build
XDG_DATA_DIRS="$PWD/_install/share:$XDG_DATA_DIRS" _install/bin/bitlog-gtk
```

`XDG_DATA_DIRS` lets the app find its settings schema and icon in `_install/`.
On first start, open a vault folder; `fixtures/sample-vault/` is one to try.
BitLog opens the same vault again on the next start.
`meson test -C _build` validates the desktop file, metainfo and settings
schema.

### Flatpak

The Flatpak manifest is `build-aux/dev.bitlog.BitLog.json`. It needs
`flatpak-builder`, the GNOME 50 SDK and the Rust extension from Flathub:

```sh
flatpak install flathub org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean _flatpak build-aux/dev.bitlog.BitLog.json
flatpak run dev.bitlog.BitLog
```

The build runs without network access and takes the crates from
`build-aux/cargo-sources.json`. After every change to `Cargo.lock`,
regenerate that file with
[flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo),
for example with `uv`, which installs the script's dependencies:

```sh
curl -LO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
uv run flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
rm flatpak-cargo-generator.py
```

The app gets access to the home folder, so that a vault can live anywhere in
it.

For a release, `build-aux/flatpak-bundle.sh` builds the Flatpak and packs
it into `_dist/bitlog-<version>.flatpak`, which goes into the release's
assets.

## Command line

The command `bitlog` works on a vault from the terminal. Build and install
it with Cargo:

```sh
cargo install --path crates/bitlog-cli
```

`bitlog init` creates a new vault in the current folder, or in the folder
given with `--vault`. The folder may only hold hidden files such as `.git`.

All other commands work on an existing vault. It is the folder given with
`--vault`, or else the one in the environment variable `BITLOG_VAULT`, or
else the current folder or the closest folder above it that holds a
`bitlog.toml`.

```sh
bitlog today              # today's blocks with their ids, working time and location
bitlog day 2026-09-23     # the same for another day
bitlog standup            # the last day with work and today, with your commits, to paste into a chat

bitlog block add 09:00-10:30 webshop "Checkout flow"
bitlog block edit k7f3 --time 09:00-11:00 --title "Checkout and cart"
bitlog block note k7f3    # edit the block's text in $VISUAL or $EDITOR
bitlog block rm k7f3 --move-text
bitlog set --location office

bitlog project list
bitlog project add client-portal --name "Client portal" --color ff7800
bitlog project edit client-portal --status archived
bitlog project edit client-portal --repo ~/code/client-portal   # only on this device
bitlog project rename client-portal portal   # also in all days and notes
bitlog log client-portal --limit 5   # latest commits of that repository

bitlog task list --all    # open tasks with their ids, then the finished ones
bitlog task add "Renew the TLS certificate" --due 2026-09-30
bitlog task done h4c8     # also: drop, reopen
bitlog task edit h4c8 --title "Renew the certificates" --no-due
bitlog task move h4c8 1   # to the top of the open tasks
bitlog task archive       # move finished tasks to tasks-archive-YYYY.toml

bitlog search release deploy     # blocks, day notes, notes and tasks holding both words
bitlog search '"release notes"'  # the words as written, one after the other
bitlog stats              # time per project this month, and remote work days
bitlog stats --week       # also --year, or --from 2026-09-01 --to 2026-09-30

bitlog export blocks --from 2026-09-01 --to 2026-09-30   # CSV in exports/, all without dates
bitlog export week --date 2026-09-23                     # Markdown report of that week
bitlog export remote                                     # remote work days per year as CSV

bitlog doctor             # sync conflict copies, unknown projects, overlaps,
                            # headings in texts, broken wiki links in notes
bitlog doctor --fix       # merge conflict copies without contradictions,
                            # escape those headings so they read as text
```

These commands change today unless `--date 2026-09-23` names another day.

`search` and `stats` use an index of the vault in `.bitlog/index.sqlite`,
which they create and bring up to date by themselves. It is never synced and
can be deleted at any time.

`bitlog --help` lists all commands.

## License

BitLog is licensed under the [GNU General Public License v3.0 or later](LICENSE).
