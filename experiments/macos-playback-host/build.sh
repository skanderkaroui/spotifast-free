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
printf '%s\n' "$app_path"
