#!/bin/sh
# End-to-end check of Free playback through Spotifast itself: launches a
# debug build with the helper, waits for the saved sign-in, then plays
# through Spotifast's own control commands and reads back what it shows.
# Needs a Free account already signed in once. Usage: app-check.sh OUTPUT_DIR
set -u
out=${1:?output directory}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app="$root/target/debug/spotifast"
log="$out/spotifast.log"
mkdir -p "$out"
# Never take over a session someone is using: the check would close their
# Spotifast, and one Spotify account plays on only one player at a time.
if pgrep -f "target/debug/spotifast|SpotifastPlaybackProbe.app/Contents/MacOS" >/dev/null; then
    echo "SKIP Spotifast or its helper is running; quit it first, then run this check."
    exit 2
fi
RUST_LOG=spotifast=info sh "$root/experiments/run-free.sh" > "$log" 2>&1 &
i=0
until grep -q "web player helper\|credential store did not respond" "$log"; do
    i=$((i + 1)); [ "$i" -lt 60 ] || break; sleep 1
done
if ! grep -q "free account: local playback through the web player helper" "$log"; then
    echo "FAIL sign-in: Spotifast did not route this account to the helper"
    grep -v "rate limit\|cooldown" "$log" | tail -5
    exit 1
fi
echo "PASS sign-in: $(grep -o 'account plan[^$]*' "$log" | head -1)"
failed=0
# Passes when Spotifast's now-playing line contains the expected text.
check() {
    shown=$("$app" now-playing 2>&1)
    case "$shown" in
        *"$1"*) echo "PASS $2: $shown" ;;
        *) echo "FAIL $2: expected '$1', shown '$shown'"; failed=1 ;;
    esac
}
"$app" play-uri spotify:album:4yP0hdKOZPNshxUOjY0cZj; sleep 18 # After Hours
check "▶" "cold start plays an album"
"$app" play-uri spotify:track:7qiZfU4dY1lWllzX7mPBI3; sleep 10
check "Shape of You" "a chosen song replaces it"
"$app" seek-to 60; sleep 4
check "[1:0" "seek to 1:00"
"$app" repeat context; sleep 3
"$app" pause; sleep 3
check "⏸" "pause"
pkill -f "SpotifastPlaybackProbe.app/Contents/MacOS"; sleep 3
"$app" play-uri spotify:album:4yP0hdKOZPNshxUOjY0cZj; sleep 18
check "▶" "a new helper starts after the last one quit"
"$app" pause; sleep 2
exit "$failed"
