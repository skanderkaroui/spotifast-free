//! What the account's playback can do here, in one place.
//!
//! Premium plays through librespot and Spotifast's own audio pipeline, so it
//! can do everything. Spotify Free plays through Spotify's web player in a
//! hidden helper (`web_player`): its sound never passes through Spotifast,
//! and Spotify's server restricts some controls at times (an ad, the first
//! song of a radio). Every Free difference is listed here, with the
//! evidence that decided it, so the UI asks one question instead of
//! checking "is this Free?" in many places.
//!
//! Evidence (Free account through the helper, 2026-10-14):
//! - Tested live through the helper: play a playlist or album from a song,
//!   a list of songs, next, previous, seek, shuffle, repeat, pause, resume,
//!   volume (`connect/volume` PUT, accepted and applied), add to queue and
//!   play next (both accepted; the song appears in Spotify's upcoming
//!   list), radio (`inspiredby-mix/v2/seed_to_playlist`, then a play of the
//!   playlist it names). No `disallow_*` restriction was set on these
//!   outside ads.
//! - Read in open.spotify.com's code (REA): no playback control checks the
//!   plan; each follows Spotify's `restrictions`. Equalizer, sound quality,
//!   normalisation, crossfade and the sleep timer are not offered by the
//!   web player to anyone.
//! - From Spotifast's own design: the helper's sound plays inside WebKit,
//!   so the equalizer, balance, mono, visualizers and output device cannot
//!   act on it, and librespot settings do not reach it.

/// Which engine plays on this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// librespot with Spotifast's audio pipeline (Premium), or nothing yet.
    #[default]
    Librespot,
    /// Spotify's web player in the Free helper.
    FreeWebPlayer,
}

/// Spotify's live restrictions on the playing item (`disallow_*_reasons`
/// that are non-empty). Default: nothing restricted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Restrictions {
    pub skip_next: bool,
    pub skip_prev: bool,
    pub seek: bool,
    pub shuffle: bool,
    pub repeat: bool,
    pub queue: bool,
}

/// What the controls may offer right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub skip_next: bool,
    pub skip_prev: bool,
    pub seek: bool,
    pub shuffle: bool,
    pub repeat: bool,
    pub volume: bool,
    /// Add to queue and play next.
    pub queue_add: bool,
    /// Clear, reorder and remove queued songs. Spotify's queue for the web
    /// player only takes additions from here; Spotifast's clear-and-re-add
    /// would duplicate songs in it.
    pub queue_edit: bool,
    /// Spotify's own upcoming songs are known (from its playback state)
    /// rather than fetched from the Web API.
    pub queue_from_state: bool,
    pub radio: bool,
    /// Move playback to another device, Cast, Sonos or a network receiver.
    pub devices: bool,
    /// The equalizer, balance and mono.
    pub sound_effects: bool,
    /// Visualizers and MilkDrop, which read Spotifast's own audio.
    pub visualizers: bool,
    /// librespot's settings: device name, quality, normalisation, autoplay,
    /// gapless, buffer, cache, reconnect and restart.
    pub engine_settings: bool,
    /// Upcoming-album countdowns and pre-saves, which use librespot's session.
    pub prerelease: bool,
}

impl Capabilities {
    pub fn new(engine: Engine, restrictions: Restrictions, in_ad: bool) -> Self {
        match engine {
            Engine::Librespot => Self {
                skip_next: true,
                skip_prev: true,
                seek: true,
                shuffle: true,
                repeat: true,
                volume: true,
                queue_add: true,
                queue_edit: true,
                queue_from_state: false,
                radio: true,
                devices: true,
                sound_effects: true,
                visualizers: true,
                engine_settings: true,
                prerelease: true,
            },
            Engine::FreeWebPlayer => Self {
                // Spotify holds skips and seeks during an ad.
                skip_next: !in_ad && !restrictions.skip_next,
                skip_prev: !in_ad && !restrictions.skip_prev,
                seek: !in_ad && !restrictions.seek,
                shuffle: !restrictions.shuffle,
                repeat: !restrictions.repeat,
                volume: true,
                queue_add: !restrictions.queue,
                queue_edit: false,
                queue_from_state: true,
                radio: true,
                devices: false,
                sound_effects: false,
                visualizers: false,
                engine_settings: false,
                prerelease: false,
            },
        }
    }
}

/// Whether `uri` names something Spotify can play, like or look up. The
/// Free helper shows an ad as `spotifast:ad` (or Spotify's own
/// `spotify:ad:` id) and, for a moment after an ad, a song by name as
/// `web-player:...`; none of those is a Spotify item.
pub fn is_spotify_item(uri: &str) -> bool {
    uri.starts_with("spotify:")
        && !uri.starts_with("spotify:local:")
        && !uri.starts_with("spotify:ad:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premium_can_do_everything() {
        let caps = Capabilities::new(Engine::Librespot, Restrictions::default(), false);
        assert!(caps.skip_next && caps.seek && caps.volume && caps.queue_edit);
        assert!(caps.devices && caps.sound_effects && caps.visualizers);
        assert!(caps.engine_settings && caps.prerelease && caps.radio);
        assert!(!caps.queue_from_state);
        // Premium ignores the helper's restrictions entirely.
        let restricted = Restrictions {
            skip_next: true,
            seek: true,
            ..Restrictions::default()
        };
        assert_eq!(Capabilities::new(Engine::Librespot, restricted, true), caps);
    }

    #[test]
    fn free_offers_what_spotify_allows_and_hides_the_rest() {
        let caps = Capabilities::new(Engine::FreeWebPlayer, Restrictions::default(), false);
        assert!(caps.skip_next && caps.skip_prev && caps.seek);
        assert!(caps.shuffle && caps.repeat && caps.volume);
        assert!(caps.queue_add && caps.queue_from_state && caps.radio);
        assert!(!caps.queue_edit);
        assert!(!caps.devices && !caps.sound_effects && !caps.visualizers);
        assert!(!caps.engine_settings && !caps.prerelease);
    }

    #[test]
    fn free_follows_spotifys_restrictions_and_ads() {
        let ad = Capabilities::new(Engine::FreeWebPlayer, Restrictions::default(), true);
        assert!(!ad.skip_next && !ad.skip_prev && !ad.seek);
        let restricted = Restrictions {
            shuffle: true,
            queue: true,
            skip_prev: true,
            ..Restrictions::default()
        };
        let caps = Capabilities::new(Engine::FreeWebPlayer, restricted, false);
        assert!(!caps.shuffle && !caps.queue_add && !caps.skip_prev);
        assert!(caps.skip_next && caps.repeat);
    }

    #[test]
    fn only_spotify_items_count() {
        assert!(is_spotify_item("spotify:track:abc"));
        assert!(!is_spotify_item("spotifast:ad"));
        assert!(!is_spotify_item("spotify:ad:0123456789abcdef"));
        assert!(!is_spotify_item("web-player:Artist:Title"));
        assert!(!is_spotify_item("spotify:local:a:b:c:1"));
    }
}
