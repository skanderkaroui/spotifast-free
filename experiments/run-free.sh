#!/bin/sh
# Runs a debug build of Spotifast with the Free playback helper.
# Build first: cargo build --no-default-features --bin spotifast
#              sh experiments/macos-playback-host/build.sh
#
# Both are signed with the local "Spotifast Local Dev" identity when it
# exists. A build's own signature changes on every rebuild, and the Keychain
# then asks again for the saved sign-in; a fixed identity keeps "Always
# Allow" valid across rebuilds.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
helper_app="$root/.cache/webkit-playback-probe/SpotifastPlaybackProbe.app"
if security find-identity -p codesigning 2>/dev/null | grep -q "Spotifast Local Dev"; then
    codesign --force -s "Spotifast Local Dev" "$root/target/debug/spotifast" 2>/dev/null
    codesign --force -s "Spotifast Local Dev" "$helper_app" 2>/dev/null
fi
SPOTIFAST_PLAYBACK_HELPER="$helper_app/Contents/MacOS/SpotifastPlaybackProbe" \
    exec "$root/target/debug/spotifast" "$@"
