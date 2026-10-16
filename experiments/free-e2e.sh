#!/bin/bash
# Live acceptance check of Free playback through a Spotifast that is already
# running and signed in with a Spotify Free account. Drives it through its
# public CLI and checks both what Spotifast shows (now-playing) and what the
# hidden web player did (Spotifast's log). Never starts or quits Spotifast.
# It takes over the account's playback for about a minute.
# Usage: free-e2e.sh [PLAYLIST_URI]
set -u
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app="${SPOTIFAST:-$root/target/debug/spotifast}"
log="${SPOTIFAST_LOG:-$HOME/Library/Application Support/io.github.skanderkaroui.SpotifastFree/spotifast.log}"
playlist="${1:-spotify:playlist:37i9dQZF1DXcBWIGoYBM5M}" # Today's Top Hits (public)
timeout=15

if ! pgrep -f target/debug/spotifast >/dev/null; then
    echo "Spotifast is not running: start it signed in with a Free account first." >&2
    exit 2
fi
[ -f "$log" ] || { echo "no log at $log" >&2; exit 2; }

# Only lines written after this point count: a byte offset into the log,
# and the log's own UTC timestamps as a second guard.
start_ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
size() { wc -c < "$log" | tr -d ' '; }
start_off=$(size)
# Log lines since byte offset $1, no older than the start.
since() {
    tail -c +"$(($1 + 1))" "$log" 2>/dev/null |
        awk -v t="$start_ts" 'substr($0, 2, 20) >= t'
}

results=""
failed=0
record() { # STATUS NAME DETAIL
    results="$results$1|$2|$3
"
    [ "$1" = PASS ] || failed=1
    printf '%-4s %-16s %s\n' "$1" "$2" "$3"
}

# Waits for the web player's control event for action $2 after offset $1.
# Prints the matching line; fails on a refusal or after the timeout.
wait_control() {
    local off=$1 action=$2 i=0 line
    while [ "$i" -lt $((timeout * 2)) ]; do
        line=$(since "$off" | grep -F '"event":"control"' | grep -F "\"action\":\"$action\"" | head -1)
        if [ -n "$line" ]; then
            echo "$line"
            case "$line" in *'"handled":true'*) return 0 ;; *) return 1 ;; esac
        fi
        sleep 0.5; i=$((i + 1))
    done
    return 1
}

now_playing() { "$app" now-playing 2>&1; }
# Polls now-playing until it matches the shell pattern $1. Prints the last one.
wait_shown() {
    local i=0 shown
    while :; do
        shown=$(now_playing)
        # shellcheck disable=SC2254
        case "$shown" in $1) echo "$shown"; return 0 ;; esac
        i=$((i + 1)); [ "$i" -lt $((timeout * 2)) ] || break
        sleep 0.5
    done
    echo "$shown"; return 1
}
title_of() { echo "$1" | sed -E 's/^[^ ]+ (.*)  \[[0-9:]+ \/ [0-9:]+\]$/\1/'; }
seconds_of() { echo "$1" | sed -nE 's/.*\[([0-9]+):([0-9]+) \/.*/\1 \2/p' | awk '{print $1 * 60 + $2}'; }

steps=""      # "offset name" per step, for the stray toggle scan
# Runs the CLI verb (args after the name) and waits for the control event.
step() { # NAME ACTION VERB...
    local name=$1 action=$2 off line
    shift 2
    off=$(size)
    steps="$steps$off $name
"
    if ! "$app" "$@" >/dev/null 2>&1; then
        record FAIL "$name" "spotifast $* exited non-zero"; return 1
    fi
    if line=$(wait_control "$off" "$action"); then
        last_off=$off; return 0
    fi
    record FAIL "$name" "no handled '$action' control in ${timeout}s: ${line:-nothing logged}"
    last_off=$off; return 1
}

echo "Spotifast log: $log (from $start_ts)"

# play-uri a playlist
if step play-uri play play-uri "$playlist"; then
    if shown=$(wait_shown '▶*'); then
        first=$(title_of "$shown"); record PASS play-uri "$shown"
    else
        record FAIL play-uri "not playing: $shown"
    fi
fi
first=${first:-}

if step next next next; then
    i=0
    while :; do
        shown=$(now_playing)
        # A bare spotify: URI at 0:00 is a placeholder before the metadata.
        case "$shown" in "▶ spotify:"* | *"/ 0:00]") ;; "▶"*) [ "$(title_of "$shown")" != "$first" ] && break ;; esac
        i=$((i + 1)); [ "$i" -lt $((timeout * 2)) ] || break
        sleep 0.5
    done
    if case "$shown" in "▶ spotify:"* | *"/ 0:00]") false ;; "▶"*) true ;; *) false ;; esac &&
        [ "$(title_of "$shown")" != "$first" ]; then
        # Context is not shown; a new Load would mean it was replaced.
        if since "$last_off" | grep -q 'web player command: Load'; then
            record FAIL next "reloaded instead of skipping: $shown"
        else
            record PASS next "$shown (same context, no reload)"
        fi
    else
        record FAIL next "no new track shown playing: $shown"
    fi
fi

if step previous previous previous; then
    # The skipped-from song is still showing until the state refresh lands,
    # so wait for the first song rather than for anything playing.
    i=0
    while :; do
        shown=$(now_playing)
        case "$shown" in "▶"*) [ "$(title_of "$shown")" = "$first" ] && break ;; esac
        i=$((i + 1)); [ "$i" -lt $((timeout * 2)) ] || break
        sleep 0.5
    done
    secs=$(seconds_of "$shown"); secs=${secs:-99}
    if case "$shown" in "▶"*) true ;; *) false ;; esac && [ "$(title_of "$shown")" = "$first" ]; then
        record PASS previous "$shown (back to the first song after $((i / 2))s)"
    elif case "$shown" in "▶"*) true ;; *) false ;; esac && [ "$secs" -le 5 ]; then
        record PASS previous "$shown (restarted the song rather than stepping back)"
    else
        record FAIL previous "not back on '$first': $shown"
    fi
fi

if step seek-to seek seek-to 60; then
    i=0; secs=0
    while [ "$i" -lt $((timeout * 2)) ]; do
        shown=$(now_playing); secs=$(seconds_of "$shown"); secs=${secs:-0}
        [ "$secs" -ge 58 ] && break
        sleep 0.5; i=$((i + 1))
    done
    [ "$secs" -ge 58 ] && record PASS seek-to "$shown" || record FAIL seek-to "position ${secs}s: $shown"
fi

for arg in "shuffle on" "shuffle off" "repeat track" "repeat off"; do
    # shellcheck disable=SC2086
    set -- $arg
    step "$1-$2" "$1" $arg && record PASS "$1-$2" "handled"
done

if step pause pause pause; then
    shown=$(wait_shown "⏸*") && record PASS pause "$shown" || record FAIL pause "still: $shown"
fi
if step play resume play; then
    shown=$(wait_shown "▶*") && record PASS play "$shown" || record FAIL play "still: $shown"
fi
step volume-40 volume volume 40 && record PASS volume-40 "handled"
step volume-100 volume volume 100 && record PASS volume-100 "handled"

# transfer to a device that does not exist: Spotifast must not hand it to
# the web player, and must survive it.
before=$(now_playing)
off=$(size)
steps="$steps$off transfer
"
"$app" transfer spotifast-e2e-no-such-device >/dev/null 2>&1
sleep 5
if ! pgrep -f target/debug/spotifast >/dev/null; then
    record FAIL transfer "Spotifast quit"
elif since "$off" | grep -q 'web player command'; then
    record FAIL transfer "sent: $(since "$off" | grep 'web player command' | head -1 | sed 's/.*command: //')"
elif ! shown=$(now_playing) || [ -z "$shown" ]; then
    record FAIL transfer "now-playing stopped answering"
elif [ "${before%% *}" != "${shown%% *}" ]; then
    record FAIL transfer "shown state changed with no web player command: '$before' -> '$shown'"
else
    record PASS transfer "no web player command; $shown"
fi

if step final-pause pause pause; then
    shown=$(wait_shown "⏸*") && record PASS final-pause "$shown" ||
        record FAIL final-pause "still: $shown"
fi
# Leave the music stopped even when Spotifast lost track of it: the web
# player's last media event says whether it is still sounding.
if [ "$(since "$start_off" | grep -o '"type":"\(playing\|pause\|ended\)"' | tail -1)" = '"type":"playing"' ]; then
    echo "     (web player still playing after the last step; sending play-pause to stop it)"
    off=$(size); "$app" play-pause >/dev/null 2>&1
    wait_control "$off" pause >/dev/null || echo "     (could not stop it; pause Spotify by hand)"
fi
sleep 2
end_off=$(size)

# A Toggle is expected only from the pause/play steps. Scan every slice of
# the log between steps, and the slice before the first one.
stray=""
prev_off=$start_off; prev_name="before-first-step"
scan() { # FROM TO NAME
    local n=0
    [ "$2" -gt "$1" ] && n=$(tail -c +"$(($1 + 1))" "$log" | head -c "$(($2 - $1))" |
        awk -v t="$start_ts" 'substr($0, 2, 20) >= t' | grep -c 'web player command: Toggle')
    case "$3" in pause | play | final-pause | play-pause) [ "$n" -le 1 ] && return ;; *) [ "$n" -eq 0 ] && return ;; esac
    stray="$stray $3($n)"
}
while read -r off name; do
    [ -n "$off" ] || continue
    scan "$prev_off" "$off" "$prev_name"
    prev_off=$off; prev_name=$name
done <<EOF
$steps
EOF
scan "$prev_off" "$end_off" "$prev_name"
if [ -n "$stray" ]; then
    record FAIL stray-toggle "unexpected Toggle during:$stray"
else
    record PASS stray-toggle "none"
fi

echo
printf '%-4s | %-16s | %s\n' RESULT STEP DETAIL
printf '%s' "$results" | while IFS='|' read -r s n d; do
    printf '%-6s | %-16s | %s\n' "$s" "$n" "$d"
done
exit "$failed"
