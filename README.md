<p align="center">
  <img src="docs/assets/images/logo.svg" alt="Spotifast logo" width="88" height="88">
</p>

<h1 align="center">Spotifast</h1>

<p align="center"><strong>Spotify, native and fast, for Free and Premium accounts.</strong><br>A lightweight music app built on top of <a href="https://github.com/crmne/spotifast">Spotifast</a>.</p>

<p align="center">
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#spotify-free">Spotify Free</a> ·
  <a href="https://spotifast.rocks/using-spotifast/">User guide</a> ·
  <a href="https://github.com/skanderkaroui/spotifast-free/issues">Issues</a>
</p>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Built_with-Rust-1ed760" alt="Built with Rust"></a>
  <a href="#spotify-free"><img src="https://img.shields.io/badge/Spotify_Free-macOS-24292f" alt="Spotify Free on macOS"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-1ed760" alt="MIT license"></a>
</p>

This is a fork of [Spotifast](https://github.com/crmne/spotifast) by Carmine
Paolino that adds playback for **Spotify Free** accounts. Everything else is
Spotifast: a Spotify client written in Rust with [egui](https://www.egui.rs)
and [fastframe](https://fastframe.dev) that typically uses 100–250 MB of RAM
and starts in well under a second.

- **Premium** plays through [librespot](https://github.com/librespot-org/librespot),
  exactly as in Spotifast.
- **Free** plays through Spotify's own web player, which Spotifast runs
  unchanged in a small background helper and controls from its native
  interface. On macOS for now.

The interface includes complete
[translations for all supported languages](docs/_reference/translating.md#languages-and-coverage).

![Spotifast Home with the playlist library, recommendations, queue, and player visible](docs/screenshot.png)

## Spotify Free

On a Free account, Spotifast starts a helper that loads open.spotify.com in
macOS's built-in WebKit and sends it the same Spotify Connect requests the
web player sends for its own buttons. Spotify plays every song and every ad
itself, with its own DRM and restrictions. Spotifast never skips, mutes or
blocks an ad, and never touches the audio or its keys.

| On Free | |
|---|---|
| **Works** | Play a playlist, album, song, list of songs or Liked Songs; next, previous, seek, shuffle, repeat, volume, add to queue, and Spotify's radio |
| **Ads** | Shown as the playing item; skip and seek are held while one plays, as on Spotify |
| **Hidden** | Other devices and Cast, queue editing and Play next, local files, the equalizer, visualizers and MilkDrop, playback engine settings, pre-save. Spotify plays the audio inside WebKit, so Spotifast cannot hear or route it |
| **Not yet** | Free playback on Windows and Linux. Those builds play Premium only, like Spotifast |

The helper keeps its own Spotify sign-in, separate from your browsers, and
quits after five idle minutes. How it works, its limits, and how it is
checked against Spotify's web player: [scope](experiments/SCOPE-free-playback.md)
and [helper](experiments/macos-playback-host/README.md).

## Install

This fork has no prebuilt downloads yet; the Homebrew, AUR and download-page
builds are upstream Spotifast, which plays Premium only. Build from source on
macOS 14 or later with Rust and the Xcode command-line tools:

```sh
git clone https://github.com/skanderkaroui/spotifast-free.git
cd spotifast-free
cargo build --no-default-features --bin spotifast
sh experiments/macos-playback-host/build.sh
```

`--no-default-features` leaves out MilkDrop, which needs cmake, a C++
compiler and libclang; drop it to build MilkDrop for Premium.
[Build from source](https://spotifast.rocks/getting-started/#build-from-source)
lists every dependency.

## Start listening

1. Run `sh experiments/run-free.sh` and choose **Sign in with Spotify**.
   Approve access in your browser, then return to the app.
2. **Premium:** open the device menu in the player bar and choose **Set up
   playback here**, then approve playback in your browser. Your computer
   appears as a Spotify Connect device named **Spotifast**.
3. **Free:** sign in to the helper once. Quit Spotifast, open the helper's
   window, sign in on Spotify's page, close it, and start Spotifast again:

   ```sh
   .cache/webkit-playback-probe/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe --web-player
   ```

Spotifast remembers your sign-in in the macOS Keychain. A local build is
signed again on every rebuild, so macOS asks once more after each one;
choose **Always Allow**.

## Your music, on your desktop

| Feature | What you can do |
|---|---|
| **Library and search** | Browse playlists, Liked Songs, albums, artists, and podcasts. Search the catalogue and edit playlists you own. |
| **Local files** | Premium: play the local files your playlists list from a folder you choose in Settings, at any sample rate. |
| **Spotify Connect** | Premium: play on this computer or control your other devices, including Google Cast speakers and groups. Cast devices appear as they are discovered; frequently and recently chosen outputs rise to the top. |
| **Queue** | Add songs to the queue. On Premium, **Play next** puts a song or selection first when playing on this computer. |
| **Themes** | Choose light, dark, system appearance, or custom colours, with one theme for light mode and one for dark mode that switch with the system. On Omarchy, follow your desktop theme. |
| **Desktop controls** | Use keyboard shortcuts, media keys and the command line (`spotifast play`, `next`, `now-playing`…). Start Spotifast Free with your computer, with its window or in the tray. |
| **Winamp mini player** | Use classic skins with an equalizer, playlist, and animated sound displays. Premium only for the equalizer and visuals. |
| **MilkDrop** | Watch music-reactive visuals, Premium only. See the [MilkDrop guide](https://spotifast.rocks/milkdrop/). |

Spotifast's [user guide](https://spotifast.rocks/using-spotifast/) covers
these in detail; everything in it applies here except the install steps and
the Premium requirement. Its [troubleshooting page](https://spotifast.rocks/troubleshooting/)
explains "Spotify refused the audio key" and "Spotify is being slow".

## Tests

```sh
cargo fmt --check
cargo clippy --no-default-features --all-targets -- -D warnings
cargo test --no-default-features

# The helper's page script, offline, against recorded Spotify responses
node experiments/macos-playback-host/test.mjs
node experiments/macos-playback-host/diagnostics.test.mjs

# Live, against a running Spotifast signed in with a Free account
bash experiments/free-e2e.sh
```

The Free/Premium test matrix is in `src/app/free_matrix_tests.rs`. Recorded
Spotify responses go through
`experiments/macos-playback-host/scrub-recordings.mjs` before they are
committed, so no account's songs or playlists end up in the repository.

## Contributing

Report Free playback problems in
[this fork's issues](https://github.com/skanderkaroui/spotifast-free/issues),
with the steps and Spotifast's log. Anything that also happens in Spotifast
itself belongs [upstream](https://github.com/crmne/spotifast/issues).

To look at the interface without a Spotify account, run:

```sh
cargo run --features demo -- --demo
```

Upstream Spotifast does not embed a browser engine (see its
[CONTRIBUTING.md](CONTRIBUTING.md)), so Free playback lives in this fork.
This fork follows upstream and rebases on its releases.

## Acknowledgements

Built on [Spotifast](https://github.com/crmne/spotifast) by Carmine Paolino.
Spotifast uses [librespot](https://github.com/librespot-org/librespot),
[egui](https://www.egui.rs), [fastframe](https://fastframe.dev), the [Inter](https://rsms.me/inter/)
typeface (OFL), and [Lucide](https://lucide.dev) icons (ISC).

This project is independent and is not affiliated with Spotify or with
upstream Spotifast. Spotify is a trademark of Spotify AB.

Licensed under the [MIT License](LICENSE).
