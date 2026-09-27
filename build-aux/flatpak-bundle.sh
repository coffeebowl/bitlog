#!/usr/bin/env bash
# Builds the Flatpak and packs it into a single-file bundle in _dist/,
# ready to attach to a release.
set -euo pipefail

cd "$(dirname "$0")/.."

app_id=dev.knotbook.Knotbook
version=$(sed -n "s/^  version: '\(.*\)',$/\1/p" meson.build)
bundle=_dist/knotbook-$version.flatpak

flatpak-builder --user --force-clean --repo=_repo _flatpak "build-aux/$app_id.json"
mkdir -p _dist
flatpak build-bundle _repo "$bundle" "$app_id" \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo

echo "$bundle"
