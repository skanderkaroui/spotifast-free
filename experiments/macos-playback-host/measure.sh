#!/bin/sh
# Samples RSS and CPU of the running probe and the WebKit helpers it started.
# WebKit XPC helpers are reparented to launchd, so they are attributed by
# start time: helpers launched at or after the probe belong to it.
# Usage: measure.sh [samples] [interval_seconds]
set -eu
samples=${1:-10}
interval=${2:-3}
probe=$(pgrep -f 'SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe' | head -1)
[ -n "$probe" ] || { echo "probe is not running" >&2; exit 1; }
epoch() { date -j -f '%a %b %d %T %Y' "$(ps -o lstart= -p "$1" | xargs)" +%s; }
start_epoch=$(epoch "$probe")
pids=$probe
for pid in $(pgrep -f 'com.apple.WebKit.(WebContent|GPU|Networking)'); do
    started=$(epoch "$pid") || continue
    if [ "$started" -ge "$start_epoch" ]; then pids="$pids,$pid"; fi
done
echo "pids: $pids"
i=0
while [ "$i" -lt "$samples" ]; do
    ps -o rss=,%cpu= -p "$pids" | awk '{ rss += $1; cpu += $2 } END { printf "total_rss_mb=%.0f total_cpu_pct=%.1f\n", rss / 1024, cpu }'
    i=$((i + 1))
    if [ "$i" -lt "$samples" ]; then sleep "$interval"; fi
done
