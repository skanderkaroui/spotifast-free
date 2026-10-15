#!/bin/sh
# Cold start: launches the hidden web player, sends one Connect play at once
# (the helper holds it until the page is signed in) and reports seconds to
# the first "playing" event. Usage: cold-start.sh OUTPUT_DIR
set -eu
out=${1:?output directory}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
helper="$here/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe"
mkdir -p "$out"
fifo="$out/cold.in"
log="$out/cold.jsonl"
[ -p "$fifo" ] || mkfifo "$fifo"
start=$(date +%s)
"$helper" --headless --web-player < "$fifo" > "$log" 2>/dev/null &
exec 3>"$fifo"
elapsed() { echo $(( $(date +%s) - start )); }
printf '{"cmd":"connect","action":"play","args":{"contextUri":"spotify:album:4yP0hdKOZPNshxUOjY0cZj"}}\n' >&3
until grep -q '"type":"playing"' "$log" 2>/dev/null; do
    [ "$(elapsed)" -lt 90 ] || { echo "no audio after 90 s"; printf '{"cmd":"quit"}\n' >&3; exit 1; }
    sleep 1
done
echo "audio playing after $(elapsed) s"
sleep 5
printf '{"cmd":"connect","action":"pause"}\n' >&3
sleep 2
printf '{"cmd":"quit"}\n' >&3
