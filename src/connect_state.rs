//! open.spotify.com's own Spotify Connect data: the `connect-state` cluster
//! the web player receives, derived views of it, and the command bodies the
//! web player sends. Field meanings follow the web player's bundle
//! (`vendor~web-player`): `_parseCluster`, `_parseClusterPlayerState`,
//! `_parsePlayerState`, `_getTrackWindow`, `_offsetPosition`,
//! `_generatePlayOptions`, `_sendPlayerCommand`, `_sendConnectCommand`.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use crate::capabilities::Restrictions as ControlRestrictions;
use crate::player::RepeatMode;

/// connect-state sends 64-bit numbers as strings; accept either.
fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(0),
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f.max(0.0) as u64))
            .ok_or_else(|| serde::de::Error::custom("bad number")),
        Value::String(s) => s.parse().map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "expected number, got {other}"
        ))),
    }
}

fn de_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(0.0),
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("bad number")),
        Value::String(s) => s.parse().map_err(serde::de::Error::custom),
        other => Err(serde::de::Error::custom(format!(
            "expected number, got {other}"
        ))),
    }
}

/// The connect-state cluster (`_parseCluster`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Cluster {
    #[serde(default, deserialize_with = "de_u64")]
    pub timestamp: u64,
    #[serde(default)]
    pub active_device_id: Option<String>,
    #[serde(default)]
    pub devices: HashMap<String, Device>,
    #[serde(default)]
    pub player_state: Option<PlayerState>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device_type: String,
    #[serde(default, deserialize_with = "de_u64")]
    pub volume: u64,
    #[serde(default)]
    pub can_play: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProvidedTrack {
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub shuffling_context: bool,
    #[serde(default)]
    pub repeating_context: bool,
    #[serde(default)]
    pub repeating_track: bool,
}

/// Raw `restrictions`: `disallow_*_reasons` lists. Non-list entries such as
/// `disallow_setting_modes` are kept as raw JSON.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct Restrictions(pub HashMap<String, Value>);

impl Restrictions {
    /// Reasons for `disallow_{action}_reasons`, empty when absent.
    pub fn reasons(&self, action: &str) -> Vec<&str> {
        self.0
            .get(&format!("disallow_{action}_reasons"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// Held when any reason other than the benign `not_paused`/`already_set`
    /// is present.
    pub fn held(&self, action: &str) -> bool {
        self.reasons(action)
            .iter()
            .any(|r| *r != "not_paused" && *r != "already_set")
    }
}

/// `cluster.player_state` (`_parseClusterPlayerState` / `_parsePlayerState`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PlayerState {
    #[serde(default, deserialize_with = "de_u64")]
    pub timestamp: u64,
    #[serde(default)]
    pub context_uri: String,
    #[serde(default)]
    pub track: Option<ProvidedTrack>,
    #[serde(default, deserialize_with = "de_u64")]
    pub position_as_of_timestamp: u64,
    #[serde(default, deserialize_with = "de_u64")]
    pub duration: u64,
    #[serde(default)]
    pub is_paused: bool,
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default, deserialize_with = "de_f64")]
    pub playback_speed: f64,
    #[serde(default)]
    pub options: Options,
    #[serde(default)]
    pub restrictions: Restrictions,
    #[serde(default)]
    pub next_tracks: Vec<ProvidedTrack>,
    #[serde(default)]
    pub prev_tracks: Vec<ProvidedTrack>,
}

/// The playing track as the web player shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowTrack {
    pub uri: String,
    pub title: String,
    /// `(name, uri)`; uri empty when Spotify didn't send one.
    pub artists: Vec<(String, String)>,
    pub album_title: String,
    pub album_uri: String,
    pub image_large_url: Option<String>,
    pub image_url: Option<String>,
    pub image_small_url: Option<String>,
    pub duration_ms: u64,
}

impl Cluster {
    /// Whether `own_device` is the cluster's active device.
    pub fn is_active(&self, own_device: &str) -> bool {
        self.active_device_id.as_deref() == Some(own_device)
    }
}

impl PlayerState {
    /// Current track from `track.metadata` (`_getTrackWindow`): artists come
    /// from `artist_name`/`artist_uri`, then `artist_name:1`, `artist_name:2`...
    pub fn now_track(&self) -> Option<NowTrack> {
        let t = self.track.as_ref()?;
        let m = &t.metadata;
        let get = |k: &str| m.get(k).cloned();
        let mut artists = Vec::new();
        if let Some(n) = get("artist_name") {
            artists.push((n, get("artist_uri").unwrap_or_default()));
        }
        for i in 1.. {
            let Some(n) = get(&format!("artist_name:{i}")) else {
                break;
            };
            artists.push((n, get(&format!("artist_uri:{i}")).unwrap_or_default()));
        }
        Some(NowTrack {
            uri: t.uri.clone(),
            title: get("title").unwrap_or_default(),
            artists,
            album_title: get("album_title").unwrap_or_default(),
            album_uri: get("album_uri").unwrap_or_default(),
            image_large_url: get("image_large_url"),
            image_url: get("image_url"),
            image_small_url: get("image_small_url"),
            duration_ms: self.duration,
        })
    }

    /// `options.shuffling_context` (`_parsePlayerState` `shuffle`).
    pub fn shuffle(&self) -> bool {
        self.options.shuffling_context
    }

    /// `_parsePlayerState` repeat_mode: track wins over context.
    pub fn repeat(&self) -> RepeatMode {
        if self.options.repeating_track {
            RepeatMode::Track
        } else if self.options.repeating_context {
            RepeatMode::Context
        } else {
            RepeatMode::Off
        }
    }

    /// Control restrictions, as web-player maps `disallow_*_reasons` to
    /// `canSkipNext` etc. Repeat is held only when both toggles are.
    pub fn restrictions(&self) -> ControlRestrictions {
        let r = &self.restrictions;
        ControlRestrictions {
            skip_next: r.held("skipping_next"),
            skip_prev: r.held("skipping_prev"),
            seek: r.held("seeking"),
            shuffle: r.held("toggling_shuffle"),
            repeat: r.held("toggling_repeat_context") && r.held("toggling_repeat_track"),
            queue: r.held("add_to_queue"),
        }
    }

    /// Upcoming track uris (`_getTrackWindow` next_tracks): `spotify:` items,
    /// minus delimiters and `hidden` entries, at most 50.
    pub fn upcoming(&self) -> Vec<String> {
        self.next_tracks
            .iter()
            .filter(|t| t.uri.starts_with("spotify:") && t.uri != "spotify:delimiter")
            .filter(|t| t.metadata.get("hidden").map(String::as_str) != Some("true"))
            .take(50)
            .map(|t| t.uri.clone())
            .collect()
    }

    /// `_offsetPosition`: paused uses `position_as_of_timestamp`, else adds
    /// elapsed server time times speed; clamped to `[0, duration]`.
    pub fn position_at(&self, server_now_ms: u64) -> u32 {
        let pos = self.position_as_of_timestamp as f64;
        let p = if self.is_paused {
            pos
        } else {
            pos + (server_now_ms as f64 - self.timestamp as f64) * self.playback_speed
        };
        let p = p.max(0.0);
        let p = if self.duration > 0 {
            p.min(self.duration as f64)
        } else {
            p
        };
        p.min(u32::MAX as f64) as u32
    }
}

/// `inspiredby-mix/v2/seed_to_playlist` response.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SeedToPlaylist {
    #[serde(default, rename = "mediaItems")]
    pub media_items: Vec<MediaItem>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MediaItem {
    #[serde(default)]
    pub uri: String,
}

impl SeedToPlaylist {
    pub fn playlist_uri(&self) -> Option<&str> {
        self.media_items.first().map(|m| m.uri.as_str())
    }
}

// ---- Commands (`_sendPlayerCommand` / `_sendConnectCommand`) ----

/// `connect-state/v1/player/command/from/{from}/to/{to}` (POST).
pub fn player_command_path(from: &str, to: &str) -> String {
    format!("connect-state/v1/player/command/from/{from}/to/{to}")
}

/// `connect-state/v1/connect/volume/from/{from}/to/{to}` (PUT).
pub fn volume_path(from: &str, to: &str) -> String {
    format!("connect-state/v1/connect/volume/from/{from}/to/{to}")
}

/// What to play.
#[derive(Clone, Debug)]
pub enum PlayTarget {
    /// `playURI`: `{uri, url: "context://"+uri, metadata: {}}`.
    Context(String),
    /// `playTracks`: `spotify:internal:harmony-play-tracks` with one page.
    Tracks(Vec<String>),
}

#[derive(Clone, Debug)]
pub enum SkipTo {
    TrackUri(String),
    TrackIndex(u32),
}

/// `player_options_override` fields (`_generatePlayOptions`).
#[derive(Clone, Copy, Debug, Default)]
pub struct OptionsOverride {
    pub shuffle: Option<bool>,
    pub repeat: Option<RepeatMode>,
}

#[derive(Clone, Debug)]
pub struct PlayRequest {
    pub target: PlayTarget,
    pub skip_to: Option<SkipTo>,
    pub seek_to: Option<u64>,
    pub initially_paused: Option<bool>,
    pub options_override: Option<OptionsOverride>,
    pub license: String,
    pub command_id: String,
}

fn wrap(endpoint: &str, mut fields: Map<String, Value>, command_id: &str) -> Value {
    fields.insert("endpoint".into(), json!(endpoint));
    fields.insert(
        "logging_params".into(),
        json!({"page_instance_ids": [], "interaction_ids": [], "command_id": command_id}),
    );
    json!({ "command": fields })
}

fn repeat_fields(m: &mut Map<String, Value>, repeat: RepeatMode) {
    let (ctx, track) = match repeat {
        RepeatMode::Off => (false, false),
        RepeatMode::Context => (true, false),
        RepeatMode::Track => (false, true),
    };
    m.insert("repeating_context".into(), json!(ctx));
    m.insert("repeating_track".into(), json!(track));
}

/// `play` body (`_play` + `_generatePlayOptions` + `_generatePlayOrigin`).
pub fn play_command(req: &PlayRequest) -> Value {
    let context = match &req.target {
        PlayTarget::Context(uri) => {
            json!({"uri": uri, "url": format!("context://{uri}"), "metadata": {}})
        }
        PlayTarget::Tracks(uris) => {
            let tracks: Vec<Value> = uris
                .iter()
                .map(|u| json!({"uri": u, "metadata": {}}))
                .collect();
            json!({"uri": "spotify:internal:harmony-play-tracks", "metadata": {},
                   "pages": [{"tracks": tracks}]})
        }
    };
    let mut options = Map::new();
    options.insert("license".into(), json!(req.license));
    match &req.skip_to {
        Some(SkipTo::TrackUri(u)) => {
            options.insert("skip_to".into(), json!({"track_uri": u}));
        }
        Some(SkipTo::TrackIndex(i)) => {
            options.insert("skip_to".into(), json!({"track_index": i}));
        }
        None => {}
    }
    if let Some(s) = req.seek_to {
        options.insert("seek_to".into(), json!(s));
    }
    if let Some(p) = req.initially_paused {
        options.insert("initially_paused".into(), json!(p));
    }
    if let Some(o) = req.options_override {
        let mut m = Map::new();
        if let Some(s) = o.shuffle {
            m.insert("shuffling_context".into(), json!(s));
        }
        if let Some(r) = o.repeat {
            repeat_fields(&mut m, r);
        }
        options.insert("player_options_override".into(), Value::Object(m));
    }
    let mut f = Map::new();
    f.insert("context".into(), context);
    f.insert(
        "play_origin".into(),
        json!({"feature_identifier": "harmony"}),
    );
    f.insert("options".into(), Value::Object(options));
    wrap("play", f, &req.command_id)
}

/// Field-less commands: `pause`, `resume`, `skip_next`, `skip_prev`.
pub fn simple(endpoint: &str, command_id: &str) -> Value {
    wrap(endpoint, Map::new(), command_id)
}

pub fn seek_to(value_ms: u64, command_id: &str) -> Value {
    let mut f = Map::new();
    f.insert("value".into(), json!(value_ms));
    wrap("seek_to", f, command_id)
}

pub fn set_shuffling_context(value: bool, command_id: &str) -> Value {
    let mut f = Map::new();
    f.insert("value".into(), json!(value));
    wrap("set_shuffling_context", f, command_id)
}

/// `set_options` with both repeat flags.
pub fn set_options(repeat: RepeatMode, command_id: &str) -> Value {
    let mut f = Map::new();
    repeat_fields(&mut f, repeat);
    wrap("set_options", f, command_id)
}

/// `add_to_queue`: the queue manager marks the item queued (`G(t,{queued:!0})`).
pub fn add_to_queue(uri: &str, command_id: &str) -> Value {
    let mut f = Map::new();
    f.insert(
        "track".into(),
        json!({"uri": uri, "metadata": {"is_queued": "true"}, "provider": "queue"}),
    );
    wrap("add_to_queue", f, command_id)
}

/// `PUT connect/volume` body: `round(65535 * fraction)`.
pub fn volume(fraction: f64) -> Value {
    json!({"volume": (fraction.clamp(0.0, 1.0) * 65535.0).round() as u32})
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "device1000000000000000000000000000000000";
    // The recordings' songs and playlists are placeholders from
    // experiments/macos-playback-host/scrub-recordings.mjs.
    const LIST: &str = "spotify:playlist:playlist00000000000001";
    const SONG: &str = "spotify:track:track00000000000000006";
    const QUEUED: &str = "spotify:track:track00000000000000001";

    fn load(raw: &str) -> (String, Cluster) {
        #[derive(Deserialize)]
        struct F {
            own_device: String,
            cluster: Cluster,
        }
        let f: F = serde_json::from_str(raw).unwrap();
        (f.own_device, f.cluster)
    }

    fn fixture(raw: &str) -> PlayerState {
        let (own, c) = load(raw);
        assert!(c.is_active(&own));
        assert_eq!(own, OWN);
        let p = c.player_state.unwrap();
        assert!(!p.now_track().unwrap().title.is_empty());
        p
    }

    #[test]
    fn playlist_playing() {
        let p = fixture(include_str!("testdata/connect/playlist-playing.json"));
        assert_eq!(p.context_uri, LIST);
        let t = p.now_track().unwrap();
        assert_eq!(t.uri, SONG);
        assert_eq!(t.title, "Song 3");
        assert_eq!(
            t.artists,
            vec![
                (
                    "Artist 3".into(),
                    "spotify:artist:artist0000000000000003".into()
                ),
                (
                    "Artist 4".into(),
                    "spotify:artist:artist0000000000000004".into()
                )
            ]
        );
        assert_eq!(t.album_title, "Album 3");
        assert_eq!(t.duration_ms, 210587);
        assert!(!p.is_paused && !p.shuffle());
        assert_eq!(p.repeat(), RepeatMode::Off);
        assert_eq!(p.restrictions(), ControlRestrictions::default());
        assert_eq!(p.upcoming()[0], "spotify:track:track00000000000000002");
    }

    #[test]
    fn playlist_paused() {
        let p = fixture(include_str!("testdata/connect/playlist-paused.json"));
        assert!(p.is_paused);
        assert_eq!(p.track.as_ref().unwrap().uri, SONG);
        assert_eq!(p.position_at(p.timestamp + 60_000), 7171);
    }

    #[test]
    fn shuffle_repeat_track() {
        let p = fixture(include_str!("testdata/connect/shuffle-repeat-track.json"));
        assert!(!p.is_paused);
        assert!(p.shuffle());
        assert_eq!(p.repeat(), RepeatMode::Track);
    }

    #[test]
    fn repeat_context() {
        let p = fixture(include_str!("testdata/connect/repeat-context.json"));
        assert!(!p.is_paused && !p.shuffle());
        assert_eq!(p.repeat(), RepeatMode::Context);
    }

    #[test]
    fn after_add_to_queue() {
        let p = fixture(include_str!("testdata/connect/after-add-to-queue.json"));
        assert!(!p.is_paused);
        assert_eq!(p.upcoming()[0], QUEUED);
        assert_eq!(p.next_tracks[0].provider.as_deref(), Some("queue"));
        assert!(!p.restrictions().queue);
    }

    #[test]
    fn radio_first_song() {
        let p = fixture(include_str!("testdata/connect/radio-first-song.json"));
        assert!(!p.is_paused);
        assert!(p.context_uri.starts_with("spotify:playlist:"));
        assert_eq!(p.context_uri, "spotify:playlist:playlist00000000000002");
        assert_eq!(
            p.now_track().unwrap().uri,
            "spotify:track:track00000000000000010"
        );
        assert_eq!(
            p.restrictions.reasons("skipping_prev"),
            vec!["no_prev_track"]
        );
        let r = p.restrictions();
        assert!(r.skip_prev && !r.skip_next);
    }

    #[test]
    fn track_list() {
        let p = fixture(include_str!("testdata/connect/track-list.json"));
        assert!(!p.is_paused);
        assert_eq!(p.context_uri, "spotify:internal:harmony-play-tracks");
        assert_eq!(
            p.now_track().unwrap().uri,
            "spotify:track:track00000000000000011"
        );
        let up = p.upcoming();
        assert!(up.len() <= 50);
        assert!(!up.iter().any(|u| u == "spotify:delimiter"));
    }

    #[test]
    fn position_monotonic_and_clamped() {
        let p = fixture(include_str!("testdata/connect/playlist-playing.json"));
        assert_eq!(p.position_at(p.timestamp), 1092);
        let mut last = 0;
        for dt in (0..300_000).step_by(1000) {
            let now = p.position_at(p.timestamp + dt);
            assert!(now >= last);
            last = now;
        }
        assert_eq!(last, 210587);
        assert_eq!(p.position_at(0), 0);
    }

    #[test]
    fn numbers_as_strings_or_numbers() {
        let c: Cluster = serde_json::from_str(
            r#"{"timestamp":"5","devices":{"d":{"volume":"7"}},
                "player_state":{"timestamp":6,"duration":"9","playback_speed":"1"}}"#,
        )
        .unwrap();
        assert_eq!(c.timestamp, 5);
        assert_eq!(c.devices["d"].volume, 7);
        let p = c.player_state.unwrap();
        assert_eq!((p.timestamp, p.duration, p.playback_speed), (6, 9, 1.0));
    }

    #[test]
    fn seed_to_playlist() {
        #[derive(Deserialize)]
        struct F {
            response: SeedToPlaylist,
        }
        let f: F =
            serde_json::from_str(include_str!("testdata/connect/seed-to-playlist.json")).unwrap();
        assert_eq!(
            f.response.playlist_uri(),
            Some("spotify:playlist:playlist00000000000002")
        );
    }

    fn lp() -> Value {
        json!({"page_instance_ids": [], "interaction_ids": [], "command_id": "c"})
    }

    #[test]
    fn play_context_body() {
        let v = play_command(&PlayRequest {
            target: PlayTarget::Context(LIST.into()),
            skip_to: Some(SkipTo::TrackUri(SONG.into())),
            seek_to: Some(1000),
            initially_paused: Some(false),
            options_override: Some(OptionsOverride {
                shuffle: Some(true),
                repeat: Some(RepeatMode::Context),
            }),
            license: "premium".into(),
            command_id: "c".into(),
        });
        assert_eq!(
            v,
            json!({"command": {
                "endpoint": "play",
                "context": {"uri": LIST, "url": format!("context://{LIST}"), "metadata": {}},
                "play_origin": {"feature_identifier": "harmony"},
                "options": {
                    "license": "premium",
                    "skip_to": {"track_uri": SONG},
                    "seek_to": 1000,
                    "initially_paused": false,
                    "player_options_override": {
                        "shuffling_context": true,
                        "repeating_context": true,
                        "repeating_track": false
                    }
                },
                "logging_params": lp()
            }})
        );
    }

    #[test]
    fn play_tracks_body() {
        let v = play_command(&PlayRequest {
            target: PlayTarget::Tracks(vec![SONG.into()]),
            skip_to: Some(SkipTo::TrackIndex(0)),
            seek_to: None,
            initially_paused: None,
            options_override: None,
            license: String::new(),
            command_id: "c".into(),
        });
        assert_eq!(
            v,
            json!({"command": {
                "endpoint": "play",
                "context": {"uri": "spotify:internal:harmony-play-tracks", "metadata": {},
                            "pages": [{"tracks": [{"uri": SONG, "metadata": {}}]}]},
                "play_origin": {"feature_identifier": "harmony"},
                "options": {"license": "", "skip_to": {"track_index": 0}},
                "logging_params": lp()
            }})
        );
    }

    #[test]
    fn small_command_bodies() {
        for e in ["pause", "resume", "skip_next", "skip_prev"] {
            assert_eq!(
                simple(e, "c"),
                json!({"command": {"endpoint": e, "logging_params": lp()}})
            );
        }
        assert_eq!(
            seek_to(5, "c"),
            json!({"command": {"endpoint": "seek_to", "value": 5, "logging_params": lp()}})
        );
        assert_eq!(
            set_shuffling_context(true, "c"),
            json!({"command": {"endpoint": "set_shuffling_context", "value": true,
                               "logging_params": lp()}})
        );
        assert_eq!(
            set_options(RepeatMode::Track, "c"),
            json!({"command": {"endpoint": "set_options", "repeating_context": false,
                               "repeating_track": true, "logging_params": lp()}})
        );
        assert_eq!(
            add_to_queue(QUEUED, "c"),
            json!({"command": {"endpoint": "add_to_queue",
                "track": {"uri": QUEUED, "metadata": {"is_queued": "true"}, "provider": "queue"},
                "logging_params": lp()}})
        );
        assert_eq!(volume(1.0), json!({"volume": 65535}));
        assert_eq!(volume(0.5), json!({"volume": 32768}));
        assert_eq!(
            player_command_path("a", "b"),
            "connect-state/v1/player/command/from/a/to/b"
        );
        assert_eq!(
            volume_path("a", "b"),
            "connect-state/v1/connect/volume/from/a/to/b"
        );
    }
}
