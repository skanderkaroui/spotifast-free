# Scope: Spotify Free playback in Spotifast

Status: Free playback works on macOS through the helper below. Windows and
Linux are not covered yet.

## Goal

Let a Spotify Free account play music from Spotifast's native egui interface
while keeping Spotify's own player, DRM, ads and restrictions intact. Keep the
fast native harness: no Chrome automation, no ad blocking, no DRM
circumvention, no alternate audio sources.

## Out of scope

- Removing or skipping ads, muting them, or faking account tier.
- Reimplementing Spotify's player protocol, extracting keys or decoded audio.
- EQ, visualisers and the audio cache for Free playback (the web host owns
  protected audio, so PCM is not available to Spotifast).
- Windows and Linux hosts until macOS passes every gate below.
- Upstreaming. Upstream Spotifast's `AGENTS.md` forbids a browser engine,
  so this lives in a fork unless that policy changes.

## Design

1. **One sign-in.** Keep the existing login screen. The verified profile
   already reports `product` (`src/app.rs`, `src/backend.rs`
   `on_account_checked`). `premium` keeps the librespot engine. `free`/`open`
   selects the web playback host instead of showing `PremiumNeeded`.
2. **One control surface, two transports.** UI keeps emitting the same
   `Action`s. The backend routes `PlayerCommand` (`Toggle`, `Next`,
   `Previous`, `Seek`, `Volume`, `Shuffle`, `Repeat`, `Load`) to either
   librespot or the web host.
3. **Capabilities come from Spotify, not from the tier.** Observed on one Free
   account: shuffle/repeat were disabled in one context and enabled in a
   playlist context; Previous restarts the track after 3 s. The host reports
   current restrictions and ad state, and the native controls disable what
   Spotify disables. Free mode must not reuse optimistic queue
   reconstruction or optimistic skip.
4. **Playback helper (macOS).** A separate background process
   (`experiments/macos-playback-host`, `--headless --web-player`) runs
   Spotify's unchanged full web player in a hidden `WKWebView` with its own
   website data store and Safari's user-agent suffix. Spotify then picks its
   FairPlay path. Spotifast talks to it over JSON lines on stdin/stdout.
   Commands are the Spotify Connect requests open.spotify.com sends for its
   own buttons (`play` with a context and a starting song, `skip_next`,
   `seek_to`, `set_shuffling_context`, `set_options`), sent by the page with
   its own sign-in to its own device; Spotifast's sign-in is refused there
   (403). Events report Spotify's playback state (track URI and details,
   shuffle, repeat, position), media state and ads. Started as soon as
   Spotifast knows the account is Free and kept running, so a press never
   waits on a cold start (about 6 s).
   The compact embed is **not** usable: Spotify's embed code forces 30 s
   previews when it detects Safari.
5. **Process cost is tracked, not assumed.** The host adds WebKit
   WebContent, Networking and GPU/media processes even when not visible.

## Validation gates (checked before touching `src/`, 13 Oct)

The gates as they stood when the helper was first wired into Spotifast.
Commands later moved from clicking Spotify's bar to Spotify Connect, and the
live checks (`experiments/free-e2e.sh`, `connect-check.sh`) now cover play,
next, previous, seek, shuffle, repeat, pause, volume, queue and radio.

| # | Gate | How it is checked | Status (13 Oct) |
|---|------|-------------------|-----------------|
| G1 | System WebKit on macOS offers an EME key system Spotify uses | Capability probe on `open.spotify.com` | Pass: FairPlay yes, Widevine no |
| G2 | Spotify accepts the host and plays a full Free track (not a 30 s preview) | Sign in in the visible probe, play past 0:30 | Pass for the full web player (FairPlay in GPU log, user heard audio). Embed: preview only |
| G3 | A naturally served ad plays, completes, and music resumes; controls are restricted during it | Long session, no interference | Open: two 8 s items played, not confirmed as ads |
| G4 | Commands work through the bridge: play/pause, seek, next/previous, volume, load context | `drive-web-player.sh` | Partial: play, pause, next handled. `seekto` not registered by Spotify. Previous, volume, load untested |
| G5 | Login persists across restart; logout clears the host store | `cold-start.sh` | Partial: two fresh helper launches reused the stored login, ready in 3 to 4 s, audio in 5 to 6 s. Logout untested |
| G6 | Total RSS and CPU (helper plus its WebKit processes) idle, paused and playing | `measure.sh` | One run: 719 MB idle, 766 MB playing at 5 to 7% CPU, 671 MB paused. Chrome tab estimate was 512 MB |

If G1 or G2 fails, stop this approach. Do not fall back to Chrome
automation or change entitlement checks.

## Implementation steps after the gates pass

1. `src/web_player.rs` (macOS `cfg`): spawns and stops the helper, writes
   commands, parses events into an `EngineEvent`-compatible state stream.
2. `backend.rs`: in `on_account_checked`, start the web host for Free instead
   of emitting `PREMIUM_NEEDED`; route `PlayerCommand`s by transport.
3. `app.rs`: drop the Free `PremiumNeeded` dialog when the host is available;
   bind control enabled state to reported restrictions and ad state.
4. Tests: transport selection by tier, restriction-to-control mapping, ad
   state disabling skip/seek. Keep the other two targets compiling.

## Responsiveness and memory (24 Oct)

Goal: every press answers at once because the helper and its Spotify
connection stay alive, with the helper as light as can be measured. Memory
is `footprint` (what Activity Monitor shows), helper plus the WebKit
processes it starts, 5 samples per run.

| Change | Why | Measured before | Measured after |
|---|---|---|---|
| Start the helper once the plan is Free; no idle quit | The first press waited on a cold start, and nothing recorded why the helper quit after 5 idle minutes | First press about 6 s | First press accepted in 0.38 s, through the app |
| On 404/410, re-read the device list; if the device is gone, reload the page and replay the press | A long-lived helper got 404/410 on 34 of 35 presses after its connection to Spotify dropped, until restarted. Spotify's page registers its device again on each load | Stuck until restart | Faked loss against real Spotify: one reload, press accepted 4.7 s later |
| Smaller hidden window (400x300) | Less to draw | 454 MB idle | 481 MB idle. Worse, dropped |
| Load `open.spotify.com/preferences` instead of the home feed | Same web player, player bar and sign-in, without the feed's artwork nobody sees | Playing: 474 to 608 MB (page process 371 to 404 MB) | Playing: 409 to 422 MB (page process 288 to 293 MB). 15 commands accepted in 100 to 300 ms. Signed-out state detected. A real ad break (two ads, 62 s) reported from start to end, then music resumed |
