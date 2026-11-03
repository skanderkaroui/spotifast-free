#!/bin/sh
# Builds the Linux Free playback helper download: castLabs Electron (Chromium
# with Widevine) renamed spotifast-free-helper, with this folder and the
# shared Diagnostics.js as its app. Spotifast finds it unpacked in
# ~/.local/share/spotifast-free-helper (or beside the spotifast binary).
#
#   sh experiments/linux-playback-host/package.sh v0.18.0 x64 dist
set -eu
tag=$1
arch=${2:-x64}      # castLabs Electron's name: x64 or arm64
out=${3:-dist}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
electron=$(sed -n 's/.*castlabs\/electron-releases#v\([^"]*\)".*/\1/p' "$here/package.json")
case "$arch" in x64) cpu=x86_64 ;; arm64) cpu=aarch64 ;; *) echo "arch: x64 or arm64" >&2; exit 2 ;; esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
zip="electron-v${electron}-linux-${arch}.zip"
curl --fail --location --retry 3 --silent --show-error \
  "https://github.com/castlabs/electron-releases/releases/download/v${electron}/${zip}" -o "$work/$zip"
helper="$work/spotifast-free-helper"
mkdir -p "$helper"
(cd "$helper" && unzip -q "$work/$zip")
mv "$helper/electron" "$helper/spotifast-free-helper"
# Electron's own sample app is not ours to ship.
rm -f "$helper/resources/default_app.asar"
mkdir -p "$helper/resources/app"
cp "$here/main.js" "$here/bridge.js" "$here/package.json" \
  "$root/experiments/macos-playback-host/Diagnostics.js" "$helper/resources/app/"
cp "$root/LICENSE" "$helper/LICENSE.spotifast"

mkdir -p "$out"
archive="$out/spotifast-${tag}-linux-free-helper-${cpu}.tar.gz"
tar -C "$work" -czf "$archive" spotifast-free-helper
echo "$archive"
