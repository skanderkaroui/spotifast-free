#!/bin/sh
# Live check of the Connect controls through a standalone hidden web player:
# plays a playlist from a chosen song, then Next, seek, shuffle, repeat,
# pause, resume and a list of songs, and checks Spotify's reported state
# after each. Needs a Spotify session stored by the helper (sign in once in
# the visible probe). Do not run while Spotifast's own helper is playing:
# one account plays on one player at a time.
# Usage: connect-check.sh OUTPUT_DIR
set -u
out=${1:?output directory}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
helper="$here/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe"
playlist=spotify:playlist:37i9dQZF1DXcBWIGoYBM5M  # Today's Top Hits (public)
queued=spotify:track:7qiZfU4dY1lWllzX7mPBI3       # Shape of You
seed=spotify:track:0VjIjW4GlUZAMYd2vXMi3b         # Blinding Lights
mkdir -p "$out"
fifo="$out/connect.in"
log="$out/connect.jsonl"
[ -p "$fifo" ] || mkfifo "$fifo"
: > "$log"
"$helper" --headless --web-player < "$fifo" > "$log" 2>/dev/null &
exec 3>"$fifo"
failed=0
send() { printf '%s\n' "$1" >&3; sleep "$2"; }
# One field of Spotify's last reported state, as JSON.
last() {
    osascript -l JavaScript -e "function run(a){return JSON.stringify(JSON.parse(a[0])['$1'])}" \
        "$(grep '"event":"state"' "$log" | tail -1)" 2>/dev/null
}
expect() {
    # Spotify reports state a few seconds after a command, sometimes more:
    # wait up to 10 s for the expected value before failing.
    for _ in 1 2 3 4 5 6 7 8 9 10; do [ "$(last "$1")" = "$2" ] && break; sleep 1; done
    got=$(last "$1")
    if [ "$got" = "$2" ]; then echo "PASS $3 ($1=$got)"; else echo "FAIL $3: $1=$got, wanted $2"; failed=1; fi
}
send "{\"cmd\":\"connect\",\"action\":\"play\",\"args\":{\"contextUri\":\"$playlist\",\"trackIndex\":2,\"shuffle\":false,\"repeat\":\"off\"}}" 15
# A cold start can take longer than 15 s before Spotify reports state.
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -n "$(last contextUri)" ] && [ "$(last contextUri)" != "null" ] && break; sleep 2; done
expect contextUri "\"$playlist\"" "a playlist plays from a chosen song"
first=$(last uri)
send '{"cmd":"connect","action":"next"}' 5
expect contextUri "\"$playlist\"" "Next stays in the playlist"
[ "$(last uri)" != "$first" ] && echo "PASS Next changed the song" || { echo "FAIL Next kept the song"; failed=1; }
send '{"cmd":"connect","action":"seek","args":{"positionMs":60000}}' 4
position=$(grep '"event":"progress"' "$log" | tail -1 | sed -n 's/.*"positionMs":\([0-9]*\).*/\1/p')
[ "${position:-0}" -ge 59000 ] && echo "PASS seek to 1:00 ($position ms)" || { echo "FAIL seek: $position ms"; failed=1; }
send '{"cmd":"connect","action":"shuffle","args":{"on":true}}' 4
expect shuffle true "shuffle on"
send '{"cmd":"connect","action":"repeat","args":{"mode":"track"}}' 4
expect repeat '"track"' "repeat one"
send '{"cmd":"connect","action":"pause"}' 4
expect paused true "pause"
send '{"cmd":"connect","action":"resume"}' 4
expect paused false "resume"
send '{"cmd":"connect","action":"shuffle","args":{"on":false}}' 3
send '{"cmd":"connect","action":"repeat","args":{"mode":"off"}}' 3
expect repeat '"off"' "repeat off"
send "{\"cmd\":\"connect\",\"action\":\"queue\",\"args\":{\"uri\":\"$queued\"}}" 4
case "$(last upcoming)" in
    "[\"$queued\""*) echo "PASS add to queue plays next" ;;
    *) echo "FAIL add to queue: upcoming=$(last upcoming | cut -c1-80)"; failed=1 ;;
esac
send '{"cmd":"connect","action":"volume","args":{"volume":32768}}' 2
send '{"cmd":"connect","action":"volume","args":{"volume":65535}}' 2
send "{\"cmd\":\"connect\",\"action\":\"radio\",\"args\":{\"seed\":\"$seed\"}}" 6
station=$(sed -n 's/.*"event":"radio".*"station":"\([^"]*\)".*/\1/p' "$log" | tail -1)
[ -n "$station" ] && echo "PASS radio found a station ($station)" || { echo "FAIL radio found no station"; failed=1; }
expect contextUri "\"$station\"" "radio plays its station"
send "{\"cmd\":\"connect\",\"action\":\"play\",\"args\":{\"tracks\":[\"$queued\",\"$seed\"],\"trackIndex\":1}}" 8
expect uri "\"$seed\"" "a list of songs plays the chosen one"
send '{"cmd":"connect","action":"pause"}' 2
send '{"cmd":"quit"}' 2
if grep -q '"handled":false' "$log"; then
    echo "FAIL a command was refused:"; grep '"handled":false' "$log" | cut -c1-200; failed=1
fi
exit "$failed"
