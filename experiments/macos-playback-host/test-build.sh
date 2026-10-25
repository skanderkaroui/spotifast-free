#!/bin/sh
# The release ships a universal Mac app, and native-packages rejects nested
# code that lacks one of its architectures. Builds the playback helper and
# checks it carries both. Usage: sh experiments/macos-playback-host/test-build.sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app=$(sh "$here/build.sh" | tail -1)
binary="$app/Contents/MacOS/SpotifastPlaybackProbe"
archs=$(lipo -archs "$binary")
for arch in arm64 x86_64; do
    case " $archs " in
        *" $arch "*) ;;
        *) echo "the helper lacks $arch: $archs" >&2; exit 1 ;;
    esac
done
codesign --verify --strict "$app"
echo "ok: helper is universal ($archs)"
