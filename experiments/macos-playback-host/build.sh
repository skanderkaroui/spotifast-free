#!/bin/sh
set -eu
probe_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app_path="$probe_dir/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app"
mkdir -p "$app_path/Contents/MacOS" "$app_path/Contents/Resources"
xcrun swiftc -O -warnings-as-errors -framework AppKit -framework WebKit \
    "$probe_dir/Probe.swift" -o "$app_path/Contents/MacOS/SpotifastPlaybackProbe"
cp "$probe_dir/Info.plist" "$app_path/Contents/Info.plist"
cp "$probe_dir/MediaCapabilities.js" "$app_path/Contents/Resources/MediaCapabilities.js"
cp "$probe_dir/Diagnostics.js" "$app_path/Contents/Resources/Diagnostics.js"
# Spotifast's icon, so the helper's window and Dock entry are recognisable.
icon_png="$probe_dir/../../packaging/macos/icon-1024.png"
if [ -f "$icon_png" ] && command -v iconutil >/dev/null; then
    iconset="$(mktemp -d)/spotifast.iconset"
    mkdir -p "$iconset"
    for size in 16 32 128 256 512; do
        sips -z $size $size "$icon_png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
        sips -z $((size * 2)) $((size * 2)) "$icon_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$iconset" -o "$app_path/Contents/Resources/spotifast.icns"
fi
printf '%s\n' "$app_path"
