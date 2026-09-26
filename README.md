# Knotbook

A daily dev log for developers, made for Linux and GNOME.

Knotbook splits the working day into 15-minute blocks and assigns them to
projects – roughly and in hindsight, without a stopwatch. Blocks can carry
Markdown notes, projects keep a small set of notes of their own. All data
lives in plain text files in an ordinary folder, the vault, which you can
sync with Syncthing, Nextcloud, Git or anything else. There is no server.
The file format is described in [docs/format-spec.md](docs/format-spec.md).

Knotbook is at an early stage of development.

## Building

Knotbook is written in Rust (stable). The app needs GTK 4.22, libadwaita 1.9
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
XDG_DATA_DIRS="$PWD/_install/share:$XDG_DATA_DIRS" _install/bin/knotbook-gtk
```

`XDG_DATA_DIRS` lets the app find its settings schema and icon in `_install/`.
On first start, open a vault folder; `fixtures/sample-vault/` is one to try.
Knotbook opens the same vault again on the next start.
`meson test -C _build` validates the desktop file, metainfo and settings
schema.

### Flatpak

The Flatpak manifest is `build-aux/dev.knotbook.Knotbook.json`. It needs
`flatpak-builder`, the GNOME 50 SDK and the Rust extension from Flathub:

```sh
flatpak install flathub org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean _flatpak build-aux/dev.knotbook.Knotbook.json
flatpak run dev.knotbook.Knotbook
```

The build runs without network access and takes the crates from
`build-aux/cargo-sources.json`. After every change to `Cargo.lock`,
regenerate that file with
[flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo),
for example with `uv`, which installs the script's dependencies:

```sh
uv run flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
```

The app gets access to the home folder, so that a vault can live anywhere in
it.

## Command line

The command `knotbook` works on a vault from the terminal. Build and install
it with Cargo:

```sh
cargo install --path crates/knotbook-cli
```

`knotbook init` creates a new vault in the current folder, or in the folder
given with `--vault`. The folder may only hold hidden files such as `.git`.
`knotbook --help` lists all commands.

## License

Knotbook is licensed under the [GNU General Public License v3.0 or later](LICENSE).
