# Building BitLog

BitLog is written in Rust (stable). The app needs GTK 4.22, libadwaita 1.9, GtkSourceView 5.20 or newer and poppler-glib, and is built with Meson and Blueprint. On Fedora:

```sh
sudo dnf install gtk4-devel libadwaita-devel gtksourceview5-devel poppler-glib-devel \
  meson ninja-build blueprint-compiler gettext desktop-file-utils appstream
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

## Flatpak

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
