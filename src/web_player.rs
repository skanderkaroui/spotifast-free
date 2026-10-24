//! Spotify Free playback on macOS through a hidden helper process.
//!
//! librespot refuses Free accounts, so for them Spotifast starts a small
//! helper (`experiments/macos-playback-host`) that runs Spotify's own web
//! player in a hidden system WebKit view. Spotify chooses its FairPlay path
//! there and serves its ads as usual. Spotifast controls it with the same
//! Spotify Connect requests open.spotify.com sends for its own buttons
//! (play a playlist from a song, next, seek, shuffle, repeat), sent by the
//! page itself with its own sign-in. Commands go one JSON object per line on
//! stdin; Spotify's playback state comes back on stdout into the same
//! `LocalState` the librespot engine reports, so the player bar, Now Playing
//! and the tray work unchanged.
//!
//! Fork-only: upstream Spotifast forbids a browser engine (see `AGENTS.md`).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::api::models::ArtistRef;
use crate::player::{LocalState, LocalTrack, Playback, PlayerCommand, RepeatMode};

/// The device id the app sees for this transport. It is never sent to the
/// Web API: Free accounts cannot use Connect commands.
pub const DEVICE_ID: &str = "spotifast-web-player";

/// Stands in for the track URI while Spotify plays an ad.
pub const AD_URI: &str = "spotifast:ad";

/// Whether the helper's current item is one of Spotify's ads.
pub fn is_ad(uri: &str) -> bool {
    uri == AD_URI
}

/// Called with every new state, from the helper's reader thread.
pub type Notify = Arc<dyn Fn(LocalState) + Send + Sync>;

/// How long `shutdown` waits for the helper to quit before killing it.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long a just-asked play or pause holds against Spotify's state reports
/// that still say the opposite. Spotify confirms within a second or two in
/// live runs; past this, its report wins (the command was refused).
const ASKED_PLAYBACK_HOLD: std::time::Duration = std::time::Duration::from_secs(4);

/// The last plan Spotify reported, saved with the account it belongs to so
/// the next launch knows at once whether to use the helper instead of
/// waiting on a rate-limited `/me`. Two lines: the account id, then `free`
/// or `premium`. A plan saved for another account, or in the old one-line
/// form, reads as unknown.
pub fn saved_plan(path: &std::path::Path, account: &str) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != account {
        return None;
    }
    match lines.next()?.trim() {
        "premium" => Some(true),
        "free" => Some(false),
        _ => None,
    }
}

/// Writes the plan through a temporary file and a rename, so a crash never
/// leaves a half-written file.
pub fn save_plan(path: &std::path::Path, account: &str, premium: bool) {
    let Some(dir) = path.parent() else { return };
    let temporary = dir.join(".plan.tmp");
    let text = format!("{account}\n{}\n", if premium { "premium" } else { "free" });
    let result = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&temporary, text))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = result {
        log::warn!("couldn't save the account plan: {error}");
    }
}

/// Deletes the saved plan on sign-out, so the next account starts unknown.
pub fn forget_plan(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("couldn't forget the account plan: {error}");
    }
}

pub struct WebPlayer {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    state: Arc<Mutex<LocalState>>,
    /// Tells the app about a change shown at once, as the reader does for
    /// Spotify's reports.
    notify: Notify,
}

/// Why a command could not go to the web player.
#[derive(Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Spotify's web player registers no handler for it.
    NotOffered(&'static str),
}

impl WebPlayer {
    /// The helper app's executable: `SPOTIFAST_PLAYBACK_HELPER`, else next to
    /// this executable inside the app bundle.
    pub fn helper_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("SPOTIFAST_PLAYBACK_HELPER") {
            return Some(PathBuf::from(path));
        }
        let exe = std::env::current_exe().ok()?;
        let bundled = exe
            .parent()?
            .join("../Helpers/SpotifastPlaybackProbe.app/Contents/MacOS/SpotifastPlaybackProbe");
        bundled.exists().then_some(bundled)
    }

    pub fn available() -> bool {
        Self::helper_path().is_some_and(|path| path.exists())
    }

    pub fn start(notify: Notify, ad_log: Option<PathBuf>) -> std::io::Result<Self> {
        let path = Self::helper_path().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "playback helper not found")
        })?;
        let mut command = Command::new(path);
        command.args(["--headless", "--web-player"]);
        Self::spawn(command, notify, ad_log)
    }

    fn spawn(
        mut command: Command,
        notify: Notify,
        ad_log: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().expect("stdin is piped")));
        let stdout = child.stdout.take().expect("stdout is piped");
        let state = Arc::new(Mutex::new(LocalState {
            connected: true,
            volume: u16::MAX,
            ..LocalState::default()
        }));
        let shared = Arc::clone(&state);
        let reader_notify = Arc::clone(&notify);
        std::thread::Builder::new()
            .name("web-player-events".into())
            .spawn(move || {
                let notify = reader_notify;
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    // What the web player did with each command, and its
                    // errors, so a failed play is visible in Spotifast's log.
                    if matches!(
                        event.get("event").and_then(|v| v.as_str()),
                        Some(
                            "control"
                                | "page-error"
                                | "page-rejection"
                                | "account"
                                | "navigation-error"
                                | "ad"
                                | "state"
                                | "radio"
                                | "device-recovery"
                        )
                    ) || (event.get("event").and_then(|v| v.as_str()) == Some("media")
                        && matches!(
                            event.get("type").and_then(|v| v.as_str()),
                            Some("playing" | "pause" | "ended")
                        ))
                    {
                        log::info!("web player: {line}");
                    }
                    if event.get("event").and_then(|v| v.as_str()) == Some("ad")
                        && let Some(path) = &ad_log
                    {
                        append_ad_log(path, &event);
                    }
                    let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                    if apply_event(&mut state, &event, Instant::now()) {
                        notify(state.clone());
                    }
                }
                // The helper quit (crashed, or killed). Playback stops, but the
                // next command starts a new one, so it stays available:
                // reporting it disconnected would hold every later play.
                let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                state.playback = Playback::Stopped;
                state.loading = false;
                state.position_at = None;
                notify(state.clone());
            })?;
        Ok(Self {
            child,
            stdin,
            state,
            notify,
        })
    }

    pub fn command(&mut self, command: &PlayerCommand) -> std::io::Result<Result<(), Unsupported>> {
        let (playing, in_ad) = {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            (
                state.playback == Playback::Playing,
                state.track.as_ref().is_some_and(|track| is_ad(&track.uri)),
            )
        };
        let lines = match helper_lines(command, playing, in_ad) {
            Ok(lines) => lines,
            Err(unsupported) => return Ok(Err(unsupported)),
        };
        let mut stdin = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        for line in &lines {
            writeln!(stdin, "{line}")?;
        }
        stdin.flush()?;
        drop(stdin);
        // Assume the change took, so the bar shows it at once; Spotify's
        // next state report corrects it if Spotify refused.
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let before = state.clone();
        match command {
            PlayerCommand::Shuffle(on) => state.shuffle = *on,
            PlayerCommand::Repeat(mode) => state.repeat = *mode,
            // Show the new play or pause at once and hold it against
            // Spotify's reports from before the command, so the next press
            // asks for the opposite.
            PlayerCommand::Toggle if !in_ad => {
                let wanted = if playing {
                    Playback::Paused
                } else {
                    Playback::Playing
                };
                let now = Instant::now();
                state.position_ms = state.position_now();
                state.position_at = (wanted == Playback::Playing).then_some(now);
                state.playback = wanted;
                state.asked_playback = Some((wanted, now));
            }
            // Loading until the new song's audio starts.
            PlayerCommand::Load(spec) if spec.play => state.loading = true,
            PlayerCommand::Radio(_) => state.loading = true,
            PlayerCommand::Volume(volume) => state.volume = *volume,
            _ => {}
        }
        // The app only hears what changed: a play shown here at once that
        // Spotify's next report merely repeats would otherwise never reach
        // it, and it would keep showing pause while Spotify plays.
        if *state != before {
            let changed = state.clone();
            drop(state);
            (self.notify)(changed);
        }
        Ok(Ok(()))
    }

    /// Whether the helper process is still running.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn shutdown(mut self) {
        {
            let mut stdin = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
            let _ = writeln!(stdin, r#"{{"cmd":"quit"}}"#);
            let _ = stdin.flush();
        }
        // The backend waits here, so a helper that hangs must not hold it:
        // give it a moment to quit, then kill it.
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        log::warn!("web player helper did not quit; killing it");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Appends one ad event to the local ad log, one JSON object per line with
/// the time it was seen. The log stays on this computer and is never sent.
fn append_ad_log(path: &std::path::Path, event: &serde_json::Value) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut entry = serde_json::json!({ "at": seconds, "state": event.get("state") });
    for key in ["title", "lengthMs", "playedMs"] {
        if let Some(value) = event.get(key) {
            entry[key] = value.clone();
        }
    }
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        })
        .and_then(|mut file| writeln!(file, "{entry}"));
    if let Err(error) = result {
        log::warn!("couldn't write the ad log: {error}");
    }
}

/// One Spotify Connect command for the page to send (see Diagnostics.js).
fn connect(action: &str, args: serde_json::Value) -> String {
    serde_json::json!({ "cmd": "connect", "action": action, "args": args }).to_string()
}

fn repeat_name(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "off",
        RepeatMode::Context => "context",
        RepeatMode::Track => "track",
    }
}

/// Translates a player command into helper lines: Spotify Connect requests
/// the web player sends for its own buttons. Pure, so it is tested without
/// a helper.
fn helper_lines(
    command: &PlayerCommand,
    playing: bool,
    in_ad: bool,
) -> Result<Vec<String>, Unsupported> {
    use serde_json::json;
    Ok(match command {
        PlayerCommand::Toggle => vec![connect(if playing { "pause" } else { "resume" }, json!({}))],
        // Spotify does not let Free listeners skip or seek an ad; the bar
        // holds those controls rather than sending commands it refuses.
        PlayerCommand::Next | PlayerCommand::Previous | PlayerCommand::Seek(_) if in_ad => {
            return Err(Unsupported::NotOffered("skipping an ad"));
        }
        PlayerCommand::Next => vec![connect("next", json!({}))],
        PlayerCommand::Previous => vec![connect("previous", json!({}))],
        PlayerCommand::Seek(position_ms) => {
            vec![connect("seek", json!({ "positionMs": position_ms }))]
        }
        PlayerCommand::Shuffle(on) => vec![connect("shuffle", json!({ "on": on }))],
        PlayerCommand::Repeat(mode) => {
            vec![connect("repeat", json!({ "mode": repeat_name(*mode) }))]
        }
        PlayerCommand::AddToQueue(uri) => vec![connect("queue", json!({ "uri": uri }))],
        // One Connect play: the playlist or album itself, starting at the
        // chosen song, so Next stays inside it; or a list of songs played
        // as its own queue.
        PlayerCommand::Load(spec) => {
            let mut args = json!({});
            if let Some(context) = &spec.context_uri {
                args["contextUri"] = context.as_str().into();
            }
            let songs: Vec<&String> = spec
                .uris
                .iter()
                .filter(|uri| uri.starts_with("spotify:"))
                .collect();
            if spec.context_uri.is_none() {
                if songs.is_empty() {
                    return Err(Unsupported::NotOffered("an empty load"));
                }
                args["tracks"] = songs
                    .iter()
                    .map(|uri| uri.as_str())
                    .collect::<Vec<_>>()
                    .into();
            }
            if let Some(track) = spec
                .offset_uri
                .as_deref()
                .filter(|uri| uri.starts_with("spotify:"))
            {
                args["trackUri"] = track.into();
            } else if let Some(index) = spec.offset_index {
                args["trackIndex"] = index.into();
            }
            if spec.position_ms > 0 {
                args["positionMs"] = spec.position_ms.into();
            }
            if !spec.play {
                args["paused"] = true.into();
            }
            if let Some(shuffle) = spec.shuffle {
                args["shuffle"] = shuffle.into();
            }
            if let Some(repeat) = spec.repeat {
                args["repeat"] = repeat_name(repeat).into();
            }
            vec![connect("play", args)]
        }
        PlayerCommand::Volume(volume) => vec![connect("volume", json!({ "volume": volume }))],
        // Only the settled value is sent: each one is a Connect round trip.
        PlayerCommand::VolumePreview(_) => vec![],
        PlayerCommand::Radio(seed) => vec![connect("radio", json!({ "seed": seed }))],
        PlayerCommand::ClearQueue => return Err(Unsupported::NotOffered("clearing the queue")),
        PlayerCommand::Transfer => return Err(Unsupported::NotOffered("transfer")),
    })
}

/// Rewrites a `state` event from Spotify's own `player_state`, when the
/// page sent it, using the types checked against recorded open.spotify.com
/// responses (`connect_state`). Events without it are used as they are.
fn normalized_state(event: &serde_json::Value) -> std::borrow::Cow<'_, serde_json::Value> {
    use crate::connect_state::PlayerState;
    let Some(player) = event
        .get("player")
        .and_then(|raw| serde_json::from_value::<PlayerState>(raw.clone()).ok())
    else {
        return std::borrow::Cow::Borrowed(event);
    };
    let mut out = event.clone();
    if let Some(track) = player.now_track() {
        out["uri"] = track.uri.into();
        out["title"] = track.title.into();
        out["album"] = track.album_title.into();
        out["art"] = track.image_large_url.or(track.image_url.clone()).into();
        out["artSmall"] = track.image_small_url.or(track.image_url).into();
        out["durationMs"] = track.duration_ms.into();
        out["artists"] = track
            .artists
            .into_iter()
            .map(|(name, uri)| serde_json::json!({ "name": name, "uri": (!uri.is_empty()).then_some(uri) }))
            .collect::<Vec<_>>()
            .into();
    }
    out["contextUri"] = player.context_uri.clone().into();
    out["shuffle"] = player.shuffle().into();
    out["repeat"] = repeat_name(player.repeat()).into();
    out["paused"] = player.is_paused.into();
    let held = player.restrictions();
    out["restrictions"] = serde_json::json!({
        "skipNext": held.skip_next, "skipPrev": held.skip_prev, "seek": held.seek,
        "shuffle": held.shuffle, "repeat": held.repeat, "queue": held.queue,
    });
    out["upcoming"] = player.upcoming().into();
    std::borrow::Cow::Owned(out)
}

/// Folds one helper event into the state. Returns whether anything the app
/// shows changed.
fn apply_event(state: &mut LocalState, event: &serde_json::Value, now: Instant) -> bool {
    let event = if event.get("event").and_then(|v| v.as_str()) == Some("state") {
        normalized_state(event)
    } else {
        std::borrow::Cow::Borrowed(event)
    };
    let event = event.as_ref();
    let str_of = |key: &str| event.get(key).and_then(|v| v.as_str());
    match str_of("event") {
        // What Spotify answered to a command. A refused one is shown as an
        // error rather than left spinning.
        Some("control") => {
            if event.get("handled").and_then(|v| v.as_bool()) == Some(true) {
                state.signing_in = false;
                return std::mem::take(&mut state.error).is_some();
            }
            // Held while the user signs in to Spotify in the helper's window,
            // which opened by itself; it runs once they are in.
            if str_of("how") == Some("signing-in") {
                state.signing_in = true;
                state.error = Some("Sign in to Spotify in the window that opened to play".into());
                return true;
            }
            let action = str_of("action").unwrap_or("command");
            state.loading = false;
            state.signing_in = false;
            state.error = Some(match str_of("how") {
                Some("not-signed-in") => "Sign in to Spotify in the playback helper first".into(),
                _ => format!("Spotify refused {action}"),
            });
            true
        }
        // Spotify's sign-in in the helper: signing in again clears the hint.
        Some("account") => match str_of("state") {
            Some("signed-in") if state.signing_in => {
                state.signing_in = false;
                state.error = None;
                true
            }
            _ => false,
        },
        // An ad Spotify is playing. The bar shows it as the track, so the
        // listener sees an ad, not the previous song; the song returns with
        // Spotify's next state report.
        Some("ad") => match str_of("state") {
            Some("start") => {
                // Spotify's bar reports the same ad once; a repeat must not
                // restart its clock.
                if state.track.as_ref().is_some_and(|track| is_ad(&track.uri)) {
                    return false;
                }
                let title = str_of("title").unwrap_or_default();
                state.track = Some(LocalTrack {
                    uri: AD_URI.into(),
                    title: if title.is_empty() {
                        "Advertisement".into()
                    } else {
                        title.to_string()
                    },
                    artists: vec![ArtistRef {
                        name: "Spotify".into(),
                        ..ArtistRef::default()
                    }],
                    album: "Advertisement".into(),
                    duration_ms: event.get("lengthMs").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    ..LocalTrack::default()
                });
                state.track_sequence = state.track_sequence.wrapping_add(1);
                state.position_ms = 0;
                state.position_at = (state.playback == Playback::Playing).then_some(now);
                true
            }
            _ => false,
        },
        // Spotify's own playback state (Connect): the playing song with its
        // real URI and details, shuffle, repeat and whether it is paused.
        Some("state") => {
            let mut changed = false;
            let shuffle = event
                .get("shuffle")
                .and_then(|v| v.as_bool())
                .unwrap_or(state.shuffle);
            let repeat = match str_of("repeat") {
                Some("context") => RepeatMode::Context,
                Some("track") => RepeatMode::Track,
                Some("off") => RepeatMode::Off,
                _ => state.repeat,
            };
            if shuffle != state.shuffle || repeat != state.repeat {
                state.shuffle = shuffle;
                state.repeat = repeat;
                changed = true;
            }
            let restricted = |key: &str| {
                event
                    .get("restrictions")
                    .and_then(|r| r.get(key))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            };
            let restrictions = crate::capabilities::Restrictions {
                skip_next: restricted("skipNext"),
                skip_prev: restricted("skipPrev"),
                seek: restricted("seek"),
                shuffle: restricted("shuffle"),
                repeat: restricted("repeat"),
                queue: restricted("queue"),
            };
            if restrictions != state.restrictions {
                state.restrictions = restrictions;
                changed = true;
            }
            if let Some(upcoming) = event.get("upcoming").and_then(|v| v.as_array()) {
                let upcoming: Vec<String> = upcoming
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                if upcoming != state.upcoming {
                    state.upcoming = upcoming;
                    changed = true;
                }
            }
            // During an ad Spotify's state still names the song it
            // interrupted; the ad stays until it ends (its "ad end" event,
            // then the song's metadata, bring the song back).
            let in_ad = state.track.as_ref().is_some_and(|track| is_ad(&track.uri));
            if let Some(uri) = str_of("uri").filter(|uri| uri.starts_with("spotify:"))
                && !in_ad
                && !state.track.as_ref().is_some_and(|track| track.uri == uri)
            {
                let artists = event
                    .get("artists")
                    .and_then(|v| v.as_array())
                    .map(|artists| {
                        artists
                            .iter()
                            .map(|artist| {
                                let uri = artist.get("uri").and_then(|v| v.as_str());
                                ArtistRef {
                                    id: uri
                                        .and_then(|uri| uri.rsplit(':').next())
                                        .map(str::to_string),
                                    name: artist
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string(),
                                    uri: uri.map(str::to_string),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                state.track = Some(LocalTrack {
                    uri: uri.to_string(),
                    title: str_of("title").unwrap_or_default().to_string(),
                    artists,
                    album: str_of("album").unwrap_or_default().to_string(),
                    art_url: str_of("art").map(str::to_string),
                    art_small_url: str_of("artSmall").map(str::to_string),
                    duration_ms: event
                        .get("durationMs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    is_episode: uri.starts_with("spotify:episode:"),
                });
                state.track_sequence = state.track_sequence.wrapping_add(1);
                state.position_ms = 0;
                state.position_at = (state.playback == Playback::Playing).then_some(now);
                changed = true;
            }
            // Spotify's own paused flag says whether its player is playing.
            // The media element can report late (its "playing" came 18 s
            // after Spotify's state in a live run) or not at all (another
            // device took over), so the state decides both ways. An ad's
            // state belongs to the song it interrupts and is left alone.
            if let Some(paused) = event.get("paused").and_then(|v| v.as_bool())
                && !state.track.as_ref().is_some_and(|track| is_ad(&track.uri))
            {
                let wanted = if paused {
                    Playback::Paused
                } else {
                    Playback::Playing
                };
                // A play or pause just asked holds until Spotify agrees, or
                // for a few seconds if it never does (refused).
                let held = match state.asked_playback {
                    Some((asked, _)) if asked == wanted => {
                        state.asked_playback = None;
                        false
                    }
                    Some((_, at)) => now.saturating_duration_since(at) < ASKED_PLAYBACK_HOLD,
                    None => false,
                };
                if !held && state.track.is_some() && state.playback != wanted {
                    state.position_ms = state.position_now();
                    state.position_at = (wanted == Playback::Playing).then_some(now);
                    state.playback = wanted;
                    changed = true;
                }
            }
            if state.playback == Playback::Stopped && state.track.is_some() {
                state.playback = Playback::Paused;
                changed = true;
            }
            changed
        }
        // Spotify's position for the playing song, on Spotify's clock. It
        // sets the length and corrects the running clock.
        Some("progress") => {
            let (Some(position), Some(duration)) = (
                event.get("positionMs").and_then(|v| v.as_u64()),
                event.get("durationMs").and_then(|v| v.as_u64()),
            ) else {
                return false;
            };
            // The ad's own length comes with the ad; Connect describes the
            // song around it.
            if state.track.as_ref().is_none_or(|track| is_ad(&track.uri)) {
                return false;
            }
            let duration = duration as u32;
            let position = (position as u32).min(duration);
            // Only a real jump moves the clock: small drift between this
            // report and the running clock would make the bar stutter.
            let drift = state.position_now().abs_diff(position);
            let Some(track) = state.track.as_mut() else {
                return false;
            };
            let length_changed = track.duration_ms != duration;
            track.duration_ms = duration;
            // A play waits on its audio; Spotify's clock moving past the
            // first second while the element plays means it is heard.
            let heard = state.loading && state.playback == Playback::Playing && position >= 1000;
            if heard {
                state.loading = false;
            }
            if length_changed || drift > 1500 {
                state.position_ms = position;
                state.position_at = (state.playback == Playback::Playing).then_some(now);
                state.seek_sequence = state.seek_sequence.wrapping_add(u64::from(drift > 1500));
                return true;
            }
            heard
        }
        // When an ad ends, Spotify's state may still name the song it
        // interrupted; the Media Session names what is heard.
        Some("now-playing") => {
            if !state.track.as_ref().is_some_and(|track| is_ad(&track.uri)) {
                return false;
            }
            let title = str_of("title").unwrap_or_default().to_string();
            if title.is_empty() {
                return false;
            }
            let artist = str_of("artist").unwrap_or_default().to_string();
            let art: Vec<String> = event
                .get("art")
                .and_then(|v| v.as_array())
                .map(|art| {
                    art.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            // The URI is filled in by Spotify's next state report, which
            // follows every new song.
            state.track = Some(LocalTrack {
                uri: format!("web-player:{artist}:{title}"),
                title,
                artists: vec![ArtistRef {
                    name: artist,
                    ..ArtistRef::default()
                }],
                album: str_of("album").unwrap_or_default().to_string(),
                art_url: art.first().cloned(),
                art_small_url: art.last().cloned(),
                duration_ms: 0,
                is_episode: false,
            });
            state.track_sequence = state.track_sequence.wrapping_add(1);
            state.position_ms = 0;
            state.position_at = (state.playback == Playback::Playing).then_some(now);
            true
        }
        // The heard media element: whether audio is playing right now.
        Some("media") => {
            let playback = match str_of("type") {
                Some("playing") => Playback::Playing,
                Some("pause") | Some("ended") => Playback::Paused,
                Some("waiting") => {
                    // Spotify keeps several media elements; one that waits
                    // and never plays must not hold the spinner over the
                    // song that is playing.
                    if state.playback == Playback::Playing {
                        return false;
                    }
                    state.loading = true;
                    return true;
                }
                _ => return false,
            };
            let changed = playback != state.playback || state.loading;
            state.loading = false;
            state.position_ms = state.position_now();
            state.position_at = (playback == Playback::Playing).then_some(now);
            state.playback = playback;
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::LoadSpec;
    use serde_json::json;

    fn command(line: &str) -> serde_json::Value {
        serde_json::from_str(line).unwrap()
    }

    fn one(command_in: &PlayerCommand) -> serde_json::Value {
        let lines = helper_lines(command_in, false, false).unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        command(&lines[0])
    }

    fn state_event(uri: &str, title: &str) -> serde_json::Value {
        json!({
            "event": "state", "uri": uri, "contextUri": "spotify:playlist:p",
            "title": title, "artists": [{"name": "Artist", "uri": "spotify:artist:a"}],
            "album": "Album", "art": "https://i.scdn.co/large", "artSmall": "https://i.scdn.co/small",
            "durationMs": 200000, "shuffle": false, "repeat": "off", "paused": false
        })
    }

    /// A helper that ignores `quit` (a page that never loaded, a hang) is
    /// killed after the grace period instead of holding the backend.
    #[test]
    fn shutdown_kills_a_helper_that_does_not_quit() {
        let mut command = Command::new("sleep");
        command.arg("60");
        let player = WebPlayer::spawn(command, Arc::new(|_| {}), None).unwrap();
        let started = Instant::now();
        player.shutdown();
        assert!(started.elapsed() < SHUTDOWN_GRACE + std::time::Duration::from_secs(2));
    }

    /// Live run on the packaged app: pause, then play a second later, sent
    /// pause twice. Spotify's reports from before the pause still said
    /// playing, so the second press read "playing" and paused again.
    #[test]
    fn a_quick_pause_then_play_asks_for_pause_then_resume() {
        let player_state = |paused: bool| {
            let mut event = state_event("spotify:track:t", "Song");
            event["paused"] = json!(paused);
            event
        };
        // A helper that reads commands and never answers, like Spotify
        // before its first report after the command.
        let mut player = WebPlayer::spawn(Command::new("cat"), Arc::new(|_| {}), None).unwrap();
        let now = Instant::now();
        {
            let mut state = player.state.lock().unwrap();
            apply_event(&mut state, &player_state(false), now);
            assert_eq!(state.playback, Playback::Playing);
        }
        player.command(&PlayerCommand::Toggle).unwrap().unwrap();
        {
            let mut state = player.state.lock().unwrap();
            assert_eq!(state.playback, Playback::Paused, "shown paused at once");
            // A report from before the pause arrives: still playing.
            apply_event(&mut state, &player_state(false), Instant::now());
            assert_eq!(
                state.playback,
                Playback::Paused,
                "a stale report does not undo the pause"
            );
        }
        // So the next press asks to resume.
        let playing = player.state.lock().unwrap().playback == Playback::Playing;
        assert!(
            helper_lines(&PlayerCommand::Toggle, playing, false).unwrap()[0].contains("\"resume\"")
        );
        // Spotify confirms the pause; then it is no longer held.
        {
            let mut state = player.state.lock().unwrap();
            apply_event(&mut state, &player_state(true), Instant::now());
            assert!(state.asked_playback.is_none());
            // A pause Spotify refuses: after the hold, its report wins.
            state.asked_playback = Some((Playback::Playing, Instant::now() - ASKED_PLAYBACK_HOLD));
            apply_event(&mut state, &player_state(true), Instant::now());
            assert_eq!(state.playback, Playback::Paused);
        }
        player.shutdown();
    }

    /// Live runs on 0.14: pause then play, and Spotify's reports for the
    /// play said nothing new (the helper already showed playing), so the
    /// app kept the paused copy it was last sent and showed pause while
    /// Spotify played. The play or pause shown at once reaches the app too.
    #[test]
    fn a_play_or_pause_shown_at_once_reaches_the_app() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&sent);
        let mut player = WebPlayer::spawn(
            Command::new("cat"),
            Arc::new(move |state: LocalState| seen.lock().unwrap().push(state.playback)),
            None,
        )
        .unwrap();
        {
            let mut state = player.state.lock().unwrap();
            let mut event = state_event("spotify:track:t", "Song");
            event["paused"] = json!(true);
            apply_event(&mut state, &event, Instant::now());
        }
        player.command(&PlayerCommand::Toggle).unwrap().unwrap();
        assert_eq!(sent.lock().unwrap().last(), Some(&Playback::Playing));
        player.command(&PlayerCommand::Toggle).unwrap().unwrap();
        assert_eq!(sent.lock().unwrap().last(), Some(&Playback::Paused));
        player.shutdown();
    }

    /// Clicking a song in a playlist plays the playlist itself from that
    /// song, so Next stays inside it.
    #[test]
    fn a_song_in_a_playlist_plays_inside_the_playlist() {
        let sent = one(&PlayerCommand::Load(LoadSpec {
            context_uri: Some("spotify:playlist:p".into()),
            offset_uri: Some("spotify:track:t".into()),
            play: true,
            repeat: Some(RepeatMode::Off),
            ..Default::default()
        }));
        assert_eq!(sent["cmd"], "connect");
        assert_eq!(sent["action"], "play");
        assert_eq!(sent["args"]["contextUri"], "spotify:playlist:p");
        assert_eq!(sent["args"]["trackUri"], "spotify:track:t");
        assert_eq!(sent["args"]["repeat"], "off");
        assert!(sent["args"].get("tracks").is_none());
        assert!(sent["args"].get("paused").is_none());
    }

    #[test]
    fn a_context_by_index_starts_at_that_index() {
        let sent = one(&PlayerCommand::Load(LoadSpec {
            context_uri: Some("spotify:album:a".into()),
            offset_index: Some(3),
            position_ms: 42_000,
            play: true,
            ..Default::default()
        }));
        assert_eq!(sent["args"]["trackIndex"], 3);
        assert_eq!(sent["args"]["positionMs"], 42_000);
    }

    /// A list of songs without a playlist plays as its own queue.
    #[test]
    fn a_track_list_plays_as_its_own_queue() {
        let sent = one(&PlayerCommand::Load(LoadSpec {
            uris: vec!["spotify:track:a".into(), "spotify:track:b".into()],
            offset_index: Some(1),
            play: true,
            ..Default::default()
        }));
        assert_eq!(
            sent["args"]["tracks"],
            json!(["spotify:track:a", "spotify:track:b"])
        );
        assert_eq!(sent["args"]["trackIndex"], 1);
        assert!(sent["args"].get("contextUri").is_none());
    }

    /// Restoring without playing loads paused.
    #[test]
    fn a_load_without_play_starts_paused() {
        let sent = one(&PlayerCommand::Load(LoadSpec {
            uris: vec!["spotify:track:a".into()],
            play: false,
            ..Default::default()
        }));
        assert_eq!(sent["args"]["paused"], true);
    }

    #[test]
    fn an_empty_load_is_refused() {
        let spec = LoadSpec {
            uris: vec!["web-player:a:b".into(), AD_URI.into()],
            play: true,
            ..Default::default()
        };
        assert!(helper_lines(&PlayerCommand::Load(spec), false, false).is_err());
    }

    #[test]
    fn controls_are_single_connect_commands() {
        assert_eq!(one(&PlayerCommand::Next)["action"], "next");
        assert_eq!(one(&PlayerCommand::Previous)["action"], "previous");
        assert_eq!(
            one(&PlayerCommand::Seek(61_500))["args"]["positionMs"],
            61_500
        );
        assert_eq!(one(&PlayerCommand::Shuffle(true))["args"]["on"], true);
        assert_eq!(
            one(&PlayerCommand::Repeat(RepeatMode::Track))["args"]["mode"],
            "track"
        );
        assert_eq!(
            one(&PlayerCommand::AddToQueue("spotify:track:q".into()))["args"]["uri"],
            "spotify:track:q"
        );
        let pause = helper_lines(&PlayerCommand::Toggle, true, false).unwrap();
        assert_eq!(command(&pause[0])["action"], "pause");
        assert_eq!(one(&PlayerCommand::Toggle)["action"], "resume");
    }

    #[test]
    fn skip_and_seek_wait_for_the_ad() {
        for command in [
            PlayerCommand::Next,
            PlayerCommand::Previous,
            PlayerCommand::Seek(1000),
        ] {
            assert!(helper_lines(&command, true, true).is_err());
        }
        let pause = helper_lines(&PlayerCommand::Toggle, true, true).unwrap();
        assert_eq!(command(&pause[0])["action"], "pause");
    }

    #[test]
    fn clearing_stays_with_spotify() {
        assert!(helper_lines(&PlayerCommand::ClearQueue, true, false).is_err());
    }

    #[test]
    fn volume_is_a_connect_command() {
        let line = one(&PlayerCommand::Volume(32768));
        assert_eq!(line["action"], "volume");
        assert_eq!(line["args"]["volume"], 32768);
    }

    #[test]
    fn volume_preview_sends_nothing() {
        let lines = helper_lines(&PlayerCommand::VolumePreview(100), true, false).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn radio_is_seeded_by_the_track() {
        let line = one(&PlayerCommand::Radio("spotify:track:abc".into()));
        assert_eq!(line["cmd"], "connect");
        assert_eq!(line["action"], "radio");
        assert_eq!(line["args"]["seed"], "spotify:track:abc");
    }

    #[test]
    fn state_carries_restrictions_and_upcoming() {
        let mut state = LocalState::default();
        let mut event = state_event("spotify:track:a", "A");
        event["restrictions"] = json!({"skipNext": true, "seek": true, "queue": false});
        event["upcoming"] = json!(["spotify:track:b", "spotify:track:c"]);
        apply_event(&mut state, &event, Instant::now());
        assert!(state.restrictions.skip_next && state.restrictions.seek);
        assert!(!state.restrictions.skip_prev && !state.restrictions.queue);
        assert_eq!(state.upcoming, ["spotify:track:b", "spotify:track:c"]);
        // Same track, only the restrictions change: still reported.
        event["restrictions"] = json!({});
        assert!(apply_event(&mut state, &event, Instant::now()));
        assert_eq!(state.restrictions, Default::default());
    }

    #[test]
    fn restrictions_default_when_absent() {
        let mut state = LocalState::default();
        state.restrictions.shuffle = true;
        apply_event(
            &mut state,
            &state_event("spotify:track:a", "A"),
            Instant::now(),
        );
        assert_eq!(state.restrictions, Default::default());
        assert!(state.upcoming.is_empty());
    }

    /// Spotify's state names the song with its real URI and details, and
    /// sets shuffle and repeat.
    #[test]
    fn spotifys_state_is_the_playing_song() {
        let now = Instant::now();
        let mut state = LocalState::default();
        assert!(apply_event(
            &mut state,
            &state_event("spotify:track:t", "Song"),
            now
        ));
        let track = state.track.as_ref().unwrap();
        assert_eq!(track.uri, "spotify:track:t");
        assert_eq!(track.title, "Song");
        assert_eq!(track.artists[0].id.as_deref(), Some("a"));
        assert_eq!(track.album, "Album");
        assert_eq!(track.art_url.as_deref(), Some("https://i.scdn.co/large"));
        assert_eq!(track.duration_ms, 200_000);
        // Spotify's state says playing (paused:false), so it is playing.
        assert_eq!(state.playback, Playback::Playing);
        let sequence = state.track_sequence;
        assert!(!apply_event(
            &mut state,
            &state_event("spotify:track:t", "Song"),
            now
        ));
        assert_eq!(state.track_sequence, sequence, "the same song is not new");
        let mut shuffled = state_event("spotify:track:t", "Song");
        shuffled["shuffle"] = true.into();
        shuffled["repeat"] = "track".into();
        assert!(apply_event(&mut state, &shuffled, now));
        assert!(state.shuffle);
        assert_eq!(state.repeat, RepeatMode::Track);
        assert!(apply_event(
            &mut state,
            &state_event("spotify:track:u", "Next"),
            now
        ));
        assert_ne!(state.track_sequence, sequence);
    }

    /// Spotify's clock sets the length and corrects the position; a play's
    /// spinner stays until the audio is heard.
    #[test]
    fn progress_follows_spotify_and_ends_loading() {
        let now = Instant::now();
        let mut state = LocalState {
            loading: true,
            ..LocalState::default()
        };
        apply_event(&mut state, &state_event("spotify:track:t", "Song"), now);
        apply_event(&mut state, &json!({"event":"media","type":"playing"}), now);
        state.loading = true;
        assert!(apply_event(
            &mut state,
            &json!({"event":"progress","positionMs":35000,"durationMs":215000}),
            now
        ));
        assert_eq!(state.track.as_ref().unwrap().duration_ms, 215000);
        assert_eq!(state.position_ms, 35000);
        assert!(!state.loading);
        assert!(!apply_event(
            &mut state,
            &json!({"event":"progress","positionMs":35800,"durationMs":215000}),
            now
        ));
        assert_eq!(state.position_ms, 35000, "small drift keeps the clock");
    }

    /// Spotify's state decides playing and paused both ways. In a live run
    /// the media element's "playing" came 18 s after Spotify's state, and
    /// Spotifast showed paused (and skipped Previous) all that time.
    #[test]
    fn spotifys_paused_flag_sets_playing_and_paused() {
        let now = Instant::now();
        let mut state = LocalState::default();
        apply_event(&mut state, &state_event("spotify:track:t", "Song"), now);
        assert_eq!(
            state.playback,
            Playback::Playing,
            "paused:false means playing"
        );
        assert!(state.position_at.is_some(), "the clock runs");
        let mut paused = state_event("spotify:track:t", "Song");
        paused["paused"] = true.into();
        assert!(apply_event(&mut state, &paused, now));
        assert_eq!(state.playback, Playback::Paused);
        assert!(state.position_at.is_none());
        assert!(apply_event(
            &mut state,
            &state_event("spotify:track:t", "Song"),
            now
        ));
        assert_eq!(state.playback, Playback::Playing);
        // An ad keeps its own playback: the state describes the song.
        apply_event(
            &mut state,
            &json!({"event":"ad","state":"start","title":"Brand","lengthMs":15000}),
            now,
        );
        apply_event(&mut state, &paused, now);
        assert_eq!(state.playback, Playback::Playing);
        assert!(is_ad(&state.track.as_ref().unwrap().uri));
    }

    /// Spotify's own player_state from recorded open.spotify.com responses
    /// becomes what Spotifast shows: the song, playing or paused, shuffle,
    /// repeat, restrictions and upcoming songs.
    #[test]
    fn recorded_spotify_states_drive_the_app() {
        let recorded = |file: &str| -> serde_json::Value {
            let text = match file {
                "playlist-playing" => include_str!("testdata/connect/playlist-playing.json"),
                "playlist-paused" => include_str!("testdata/connect/playlist-paused.json"),
                "shuffle-repeat-track" => {
                    include_str!("testdata/connect/shuffle-repeat-track.json")
                }
                "after-add-to-queue" => include_str!("testdata/connect/after-add-to-queue.json"),
                "radio-first-song" => include_str!("testdata/connect/radio-first-song.json"),
                _ => unreachable!(),
            };
            let fixture: serde_json::Value = serde_json::from_str(text).unwrap();
            // The page sends the player_state along with a state event.
            json!({ "event": "state", "player": fixture["cluster"]["player_state"] })
        };
        let now = Instant::now();
        let shown = |file: &str| {
            let mut state = LocalState::default();
            apply_event(&mut state, &recorded(file), now);
            state
        };
        let playing = shown("playlist-playing");
        let track = playing.track.as_ref().unwrap();
        assert!(track.uri.starts_with("spotify:track:"));
        assert!(!track.title.is_empty() && !track.artists.is_empty());
        assert!(track.duration_ms > 0);
        assert_eq!(playing.playback, Playback::Playing);
        assert_eq!(
            playing.restrictions,
            crate::capabilities::Restrictions::default()
        );
        assert!(!playing.upcoming.is_empty());
        assert_eq!(shown("playlist-paused").playback, Playback::Paused);
        let shuffled = shown("shuffle-repeat-track");
        assert!(shuffled.shuffle);
        assert_eq!(shuffled.repeat, RepeatMode::Track);
        assert_eq!(
            shown("after-add-to-queue")
                .upcoming
                .first()
                .map(String::as_str),
            Some("spotify:track:track00000000000000001")
        );
        let radio = shown("radio-first-song");
        assert!(radio.restrictions.skip_prev && !radio.restrictions.skip_next);
    }

    #[test]
    fn pause_stops_the_clock() {
        let now = Instant::now();
        let mut state = LocalState::default();
        apply_event(&mut state, &state_event("spotify:track:t", "Song"), now);
        apply_event(&mut state, &json!({"event":"media","type":"playing"}), now);
        assert!(state.position_at.is_some());
        apply_event(&mut state, &json!({"event":"media","type":"pause"}), now);
        assert_eq!(state.playback, Playback::Paused);
        assert!(state.position_at.is_none());
    }

    #[test]
    fn a_refused_command_is_an_error() {
        let now = Instant::now();
        let mut state = LocalState {
            loading: true,
            ..LocalState::default()
        };
        let refused = json!({"event":"control","action":"play","handled":false,"status":403});
        assert!(apply_event(&mut state, &refused, now));
        assert!(state.error.is_some());
        assert!(!state.loading);
        let done = json!({"event":"control","action":"play","handled":true,"status":200});
        assert!(apply_event(&mut state, &done, now));
        assert!(state.error.is_none());
    }

    /// Signed out: the helper shows Spotify's sign-in and holds the play.
    /// Spotifast keeps it loading with a hint, not a refusal, and the play
    /// runs once the user is in.
    #[test]
    fn a_play_held_for_sign_in_keeps_loading_then_plays() {
        let now = Instant::now();
        let mut state = LocalState {
            loading: true,
            ..LocalState::default()
        };
        let held = json!({"event":"control","action":"play","handled":false,"how":"signing-in","waiting":true});
        assert!(apply_event(&mut state, &held, now));
        assert!(state.loading, "still loading while the user signs in");
        assert!(state.signing_in);
        assert!(
            state
                .error
                .as_deref()
                .is_some_and(|e| e.contains("Sign in"))
        );
        let signed_in = json!({"event":"account","state":"signed-in"});
        assert!(apply_event(&mut state, &signed_in, now));
        assert!(!state.signing_in && state.error.is_none());
        let done = json!({"event":"control","action":"play","handled":true,"status":200});
        apply_event(&mut state, &done, now);
        assert!(state.error.is_none());
        // A sign-in report with nothing held changes nothing.
        assert!(!apply_event(&mut state, &signed_in, now));
    }

    #[test]
    fn a_waiting_element_does_not_spin_over_a_playing_song() {
        let now = Instant::now();
        let mut state = LocalState::default();
        apply_event(&mut state, &state_event("spotify:track:t", "Song"), now);
        apply_event(&mut state, &json!({"event":"media","type":"playing"}), now);
        apply_event(&mut state, &json!({"event":"media","type":"waiting"}), now);
        assert!(!state.loading);
        assert_eq!(state.playback, Playback::Playing);
    }

    /// An ad shows as the item; a late state report of the song it
    /// interrupted does not replace it, and the song after it does.
    #[test]
    fn an_ad_shows_until_the_next_song() {
        let now = Instant::now();
        let mut state = LocalState::default();
        apply_event(&mut state, &state_event("spotify:track:t", "Song"), now);
        assert!(apply_event(
            &mut state,
            &json!({"event":"ad","state":"start","title":"Brand","lengthMs":15000}),
            now
        ));
        let ad = state.track.as_ref().unwrap();
        assert!(is_ad(&ad.uri));
        assert_eq!(ad.duration_ms, 15000);
        let sequence = state.track_sequence;
        assert!(!apply_event(
            &mut state,
            &json!({"event":"ad","state":"start","title":"Brand","lengthMs":15000}),
            now
        ));
        assert_eq!(state.track_sequence, sequence);
        assert!(!apply_event(
            &mut state,
            &json!({"event":"progress","positionMs":5000,"durationMs":200000}),
            now
        ));
        assert_eq!(state.track.as_ref().unwrap().duration_ms, 15000);
        assert!(apply_event(
            &mut state,
            &json!({"event":"now-playing","title":"Song","artist":"Artist","album":"Album"}),
            now
        ));
        assert!(!is_ad(&state.track.as_ref().unwrap().uri), "music is back");
        assert!(apply_event(
            &mut state,
            &state_event("spotify:track:t", "Song"),
            now
        ));
        assert_eq!(state.track.as_ref().unwrap().uri, "spotify:track:t");
    }

    #[test]
    fn an_untitled_ad_reads_advertisement() {
        let mut state = LocalState::default();
        apply_event(
            &mut state,
            &json!({"event":"ad","state":"start","title":null}),
            Instant::now(),
        );
        assert_eq!(state.track.as_ref().unwrap().title, "Advertisement");
    }

    #[test]
    fn ads_are_logged_locally() {
        let dir = std::env::temp_dir().join(format!("spotifast-ads-{}", std::process::id()));
        let path = dir.join("ads.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        append_ad_log(
            &path,
            &json!({"event":"ad","state":"start","title":"Brand","lengthMs":15000}),
        );
        append_ad_log(&path, &json!({"event":"ad","state":"end","playedMs":15000}));
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["state"], "start");
        assert_eq!(lines[0]["title"], "Brand");
        assert_eq!(lines[1]["playedMs"], 15000);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_plan_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("spotifast-plan-{}", std::process::id()));
        let path = dir.join("plan");
        assert_eq!(saved_plan(&path, "ana"), None);
        save_plan(&path, "ana", false);
        assert_eq!(saved_plan(&path, "ana"), Some(false));
        assert_eq!(
            saved_plan(&path, "bob"),
            None,
            "another account's plan is not used"
        );
        save_plan(&path, "ana", true);
        assert_eq!(saved_plan(&path, "ana"), Some(true));
        forget_plan(&path);
        assert_eq!(saved_plan(&path, "ana"), None, "a signed-out plan is gone");
        forget_plan(&path);
        std::fs::write(&path, "free\n").unwrap();
        assert_eq!(
            saved_plan(&path, "ana"),
            None,
            "the old unowned form is ignored"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn other_events_change_nothing() {
        let mut state = LocalState::default();
        assert!(!apply_event(
            &mut state,
            &json!({"event":"key-system"}),
            Instant::now()
        ));
        assert_eq!(state, LocalState::default());
    }
}
