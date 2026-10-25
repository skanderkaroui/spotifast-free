#!/bin/sh
set -eu
probe_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app_path="$probe_dir/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app"
mkdir -p "$app_path/Contents/MacOS" "$app_path/Contents/Resources"
# Universal, like the app that carries it: one slice per architecture,
# joined with lipo. The deployment target is the helper's own minimum.
slices=$(mktemp -d)
for arch in arm64 x86_64; do
    xcrun swiftc -O -warnings-as-errors -target "$arch-apple-macos14.0" \
        -framework AppKit -framework WebKit \
        "$probe_dir/Probe.swift" -o "$slices/$arch"
done
lipo -create "$slices/arm64" "$slices/x86_64" -output "$app_path/Contents/MacOS/SpotifastPlaybackProbe"
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
# arm64 runs only signed code; bundle.sh re-signs it inside the app.
codesign --force --sign - "$app_path" >/dev/null 2>&1
printf '%s\n' "$app_path"
