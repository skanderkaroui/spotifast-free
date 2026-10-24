# Free playback host

Spotifast starts this helper for
Spotify Free accounts on macOS (`src/web_player.rs`); see
`../SCOPE-free-playback.md` for scope and limits.

The helper loads Spotify's unchanged web player in macOS system WebKit with
its own website data store and Safari's user-agent suffix, so Spotify
chooses its FairPlay path. Sign-in happens on Spotify's own page. Browser
cookies and Spotifast credentials are not imported. No ad filtering,
license handling or key extraction is performed: the page plays every song
and every ad itself.

Spotifast Free.app ships the helper in `Contents/Helpers/`, where
`src/web_player.rs` looks for it (`packaging/macos/bundle.sh`); a debug
build finds it through `SPOTIFAST_PLAYBACK_HELPER` (`experiments/run-free.sh`).
Its bundle ID, `io.github.skanderkaroui.SpotifastFree.Helper`, keys its
Spotify sign-in.

Sign-in is automatic. The helper loads Spotify's settings page
(`open.spotify.com/preferences`): the same web player and player bar, without
the home feed's artwork, which nobody sees in the hidden window. The helper runs hidden; when Spotify's page shows
its **Log in** button (`data-testid="login-button"`), the helper shows its
window, and a command that arrives meanwhile waits (`control` with
`how: "signing-in"`). Once the account button appears
(`user-widget-link`), the window hides again and the waiting command runs.
Closing the window by hand only hides it.

Spotifast controls the page with Spotify Connect, the same requests
open.spotify.com sends for its own buttons (REA, `vendor~web-player`:
`_sendPlayerCommand`, `_generatePlayOptions`). `Diagnostics.js` sends them
from inside the page with the page's own sign-in headers, which never leave
the page, to the page's own device:
`POST https://<spclient>/connect-state/v1/player/command/from/<id>/to/<id>`.
Spotifast's own sign-in is refused there (403).

If Spotify answers 404 or 410, the page re-reads Spotify's device list, as
Spotify's own player does. If its device is still listed the command goes
once more. If it is gone (the page's live connection to Spotify dropped),
the helper reloads Spotify's page, which registers the device again, and
replays that command, at most once a minute (`device-recovery` in the log).

```sh
node experiments/macos-playback-host/test.mjs
node experiments/macos-playback-host/diagnostics.test.mjs
sh experiments/macos-playback-host/build.sh
APP=.cache/webkit-playback-probe/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe

# Visible (sign-in, debugging); it closes itself once signed in:
$APP --web-player

# Live check of every control (not while Spotifast's helper is playing):
sh experiments/macos-playback-host/connect-check.sh OUTPUT_DIR

# Seconds from launch to audio, ads over a few minutes, process cost:
sh experiments/macos-playback-host/cold-start.sh OUTPUT_DIR
sh experiments/macos-playback-host/ad-watch.sh OUTPUT_DIR 3
sh experiments/macos-playback-host/measure.sh 10 3
```

Commands, one JSON object per line on stdin:

| Command | Connect request |
|---|---|
| `{"cmd":"connect","action":"play","args":{"contextUri":"spotify:playlist:ID","trackUri":"spotify:track:ID"}}` | `play` the playlist or album from that song (`trackIndex`, `positionMs`, `paused`, `shuffle`, `repeat` optional) |
| `{"cmd":"connect","action":"play","args":{"tracks":["spotify:track:A","spotify:track:B"],"trackIndex":1}}` | `play` a list of songs as its own queue |
| `{"cmd":"connect","action":"pause"}` / `resume` / `next` / `previous` | `pause`, `resume`, `skip_next`, `skip_prev` |
| `{"cmd":"connect","action":"seek","args":{"positionMs":60000}}` | `seek_to` |
| `{"cmd":"connect","action":"shuffle","args":{"on":true}}` | `set_shuffling_context` |
| `{"cmd":"connect","action":"repeat","args":{"mode":"off"\|"context"\|"track"}}` | `set_options` |
| `{"cmd":"connect","action":"queue","args":{"uri":"spotify:track:ID"}}` | `add_to_queue` |
| `{"cmd":"media","action":"play"}` | Spotify's Media Session handler (media keys) |
| `{"cmd":"quit"}` | Exit, even before the page has loaded |

Events on stdout:

| Event | Meaning |
|---|---|
| `state` | Spotify's playback state: track URI, title, artists, album, art, length, context, shuffle, repeat, paused |
| `progress` | Position and length on Spotify's clock |
| `media` | The heard media element: `playing`, `pause`, `ended`, `waiting` (muted looping Canvas clips are ignored) |
| `now-playing` | Media Session metadata, used to leave an ad |
| `ad` | An ad started (`ad-controls` in Spotify's bar) or ended |
| `account` | Spotify's sign-in on the page: `signed-out` or `signed-in` |
| `control` | Whether Spotify accepted a command (`handled`, HTTP `status`; `how: "signing-in"` while it waits for sign-in) |
| `key-system`, `page-error` | Diagnostics |

Spotify Free ignores a list of songs sent with a playlist and plays the
playlist in its own order, so Spotifast sends a sorted view's click as the
playlist from the clicked song.

## Checking the page script against Spotify's web player

`Diagnostics.js` speaks open.spotify.com's own Connect protocol. Two checks
catch a web player update that changes it:

```sh
# Every protocol fact the page script relies on, found in Spotify's bundles
# (vendor~web-player.*.js, web-player.*.js, chunks/*.js), plus every path and
# command the page script sends. PASS/FAIL per line, exit 1 on any FAIL.
SPOTIFY_BUNDLES=/path/to/bundles node experiments/macos-playback-host/rea-conformance.mjs

# The page script against a fake page and the recorded clusters in
# src/testdata/connect/*.json (trimmed by scrub-recordings.mjs).
node experiments/macos-playback-host/diagnostics.test.mjs
```

`SPOTIFY_BUNDLES` is required: a folder with the web player's JavaScript as
open.spotify.com serves it. Re-download the bundles after a web player
update, then run both. New recordings go through `scrub-recordings.mjs`
before they are committed, so no account's songs or playlists end up here.
