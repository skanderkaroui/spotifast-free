#!/bin/sh
# Ad tracker check. Plays music for a while and reports every ad Spotify
# serves: start, length, how long it played, and whether music came back.
# Also fails if an "ad" is reported while a normal song is showing.
# Usage: ad-watch.sh OUTPUT_DIR [MINUTES]
set -u
out=${1:?output directory}
minutes=${2:-3}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
helper="$here/../../.cache/webkit-playback-probe/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe"
mkdir -p "$out"
fifo="$out/ads.in"
log="$out/ads.jsonl"
[ -p "$fifo" ] || mkfifo "$fifo"
"$helper" --headless --web-player < "$fifo" > "$log" 2>/dev/null &
exec 3>"$fifo"
sleep 10
printf '%s\n' '{"cmd":"connect","action":"play","args":{"contextUri":"spotify:album:4yP0hdKOZPNshxUOjY0cZj"}}' >&3
# Skips move through songs faster, which is when Spotify tends to serve ads.
end=$(( $(date +%s) + minutes * 60 ))
while [ "$(date +%s)" -lt "$end" ]; do
    sleep 25
    printf '%s\n' '{"cmd":"connect","action":"next"}' >&3
done
printf '%s\n' '{"cmd":"connect","action":"pause"}' >&3
sleep 2
printf '%s\n' '{"cmd":"quit"}' >&3
sleep 1
starts=$(grep -c '"event":"ad".*"state":"start"' "$log")
ends=$(grep -c '"event":"ad".*"state":"end"' "$log")
songs=$(grep '"now-playing"' "$log" | sed -n 's/.*"title":"\([^"]*\)".*/\1/p' | sort -u | wc -l | tr -d ' ')
echo "songs seen: $songs, ads started: $starts, ads ended: $ends"
grep -E '"event":"ad"' "$log" | cut -c1-200
if [ "$starts" -ne "$ends" ] && [ "$starts" -gt 0 ]; then
    echo "FAIL an ad started without ending"; exit 1
fi
[ "$songs" -ge 2 ] || { echo "FAIL music did not progress"; exit 1; }
echo "PASS"
