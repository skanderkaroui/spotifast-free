# Linux playback host

Spotify Free on Linux, the counterpart of `../macos-playback-host`. macOS
WebKit plays Spotify's DRM through FairPlay. Linux WebKitGTK offers no key
system at all, so this host runs Spotify's unchanged web player in
[castLabs Electron](https://github.com/castlabs/electron-releases) instead:
Chromium with Google's Widevine, which Chromium's component updater installs
on first launch.

Everything else is shared with macOS. `bridge.js` runs the same
`Diagnostics.js` in Spotify's page before Spotify's own scripts, and gives it
the WebKit message channel it posts to. `main.js` speaks the macOS helper's
protocol: JSON lines on stdin (`connect`, `query`, `media`, `show-sign-in`,
`quit`) and the page's events on stdout. Spotify plays every song and ad
itself, with its own DRM and restrictions. Nothing is skipped or blocked.

Spotify serves Electron's user agent a reduced page without the account
markers `Diagnostics.js` reads, so the helper presents itself as plain Chrome
(the same engine). The window is 900×600, the macOS helper's size, because a
narrower one drops the same markers.

## Run

```sh
cd experiments/linux-playback-host
npm install
./node_modules/.bin/electron . --headless   # as Spotifast starts it
./node_modules/.bin/electron .              # visible, for signing in by hand
```

The sign-in lives in its own profile (`~/.config/spotifast-free-helper`, or
`SPOTIFAST_HELPER_PROFILE`), apart from any browser.

## Check

`live-check.js` drives the helper over its protocol with a signed-in profile,
as Spotifast does. It passes when Spotify plays a full song (past 0:35, where
a preview would stop at 0:30):

```sh
SPOTIFAST_HELPER_PROFILE=... xvfb-run -a node live-check.js ./node_modules/electron/dist/electron
```

First result (Ubuntu 24.04, castLabs Electron 44.9.3, Widevine 4.10.3112.0,
a Free account): signed in, a profile query answered 200, play 200, the song
played to 43 s and paused on command.

Containers without the setuid sandbox need `--no-sandbox` on the command
line. Spotifast starts it with `--headless --no-sandbox` from a
`spotifast-free-helper` folder beside the app or in `~/.local/share`
(`src/web_player.rs`, `helper_path`): castLabs Electron renamed to
`spotifast-free-helper`, with this folder and `Diagnostics.js` in
`resources/app`. With that layout, the Free live tests in `src/web_player.rs`
and `src/free_reads.rs` pass on Linux (Ubuntu 24.04).
