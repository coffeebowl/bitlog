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

Knotbook is written in Rust (stable). The app needs GTK 4.22 and libadwaita
1.9 or newer, and is built with Meson and Blueprint. On Fedora:

```sh
sudo dnf install gtk4-devel libadwaita-devel meson ninja-build blueprint-compiler gettext
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
_install/bin/knotbook-gtk
```

## License

Knotbook is licensed under the [GNU General Public License v3.0 or later](LICENSE).
