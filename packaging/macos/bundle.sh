#!/bin/bash
# Build Spotifast.app from a GUI binary, on a macOS machine.
#
#   packaging/macos/bundle.sh <binary> <output.app> <version>
#
# Set CODESIGN_IDENTITY to sign with a Developer ID; the default is an ad-hoc
# signature, which arm64 requires before the app will launch at all.
#
# The .icns is generated here from the committed 1024px PNG, because iconutil
# only exists on macOS. The Info.plist template lives next to this script.
set -euo pipefail

binary="$1"
app="$2"
version="$3"
here="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

executable="Spotifast Free"
identifier=io.github.skanderkaroui.SpotifastFree
cp "$binary" "$app/Contents/MacOS/$executable"
chmod 755 "$app/Contents/MacOS/$executable"
# The build number has to be numbers: a release candidate's -rc1 comes off.
build="${version%%-*}"
sed -e "s/__VERSION__/$version/g" -e "s/__BUILD__/$build/g" \
    -e "s/__EXECUTABLE__/$executable/g" -e "s/__IDENTIFIER__/$identifier/g" "$here/Info.plist" \
    > "$app/Contents/Info.plist"
# Starting in the tray at login: a login item cannot pass --start-hidden, so
# SMAppService registers this agent from the bundle instead (#122).
mkdir -p "$app/Contents/Library/LaunchAgents"
cp "$here/autostart.plist" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist"

iconset="$(mktemp -d)/spotifast.iconset"
mkdir -p "$iconset"
# iconutil reads only these base sizes, each with an optional @2x. It ignores
# an icon_64x64 without saying so, so generating one is two wasted sips calls.
for size in 16 32 128 256 512; do
    sips -z $size $size "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/spotifast.icns"

# The Spotify Free playback helper goes inside the app, where
# src/web_player.rs looks for it, with the same icon. Build it first with
# experiments/macos-playback-host/build.sh; HELPER_APP overrides the path.
helper="${HELPER_APP:-$here/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app}"
if [ -d "$helper" ]; then
    mkdir -p "$app/Contents/Helpers"
    cp -R "$helper" "$app/Contents/Helpers/"
    cp "$app/Contents/Resources/spotifast.icns" \
        "$app/Contents/Helpers/SpotifastPlaybackProbe.app/Contents/Resources/spotifast.icns"
else
    echo "note: no playback helper at $helper; Free accounts will not play" >&2
fi

# arm64 refuses to launch an unsigned bundle, so sign one way or another.
# Nested code is signed before the bundle that holds it.
sign() {
    if [ -n "${CODESIGN_IDENTITY:-}" ]; then
        codesign --force --timestamp --options runtime --sign "$CODESIGN_IDENTITY" "$1"
    else
        codesign --force --sign - "$1"
    fi
}
if [ -d "$app/Contents/Helpers/SpotifastPlaybackProbe.app" ]; then
    sign "$app/Contents/Helpers/SpotifastPlaybackProbe.app"
fi
sign "$app"
codesign --verify --strict "$app"

echo "$app"
