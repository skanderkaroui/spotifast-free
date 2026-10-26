//! The rest of what a Free sign-in asks for, answered without the Web API,
//! which refuses a Free account's token: related artists, an album's songs
//! for the queue and the searches, read through Spotify's web player, and
//! the player's own state, which the helper reports itself.

use serde_json::Value;

use crate::api::ApiError;
use crate::api::models::{Artist, Page, Playlist, Track};
use crate::backend::{ApiRequest, ApiResponse};
use crate::free_reads::{self as free, SearchKind};

/// Answers `request` for a Free sign-in, or `None` when this module does
/// not cover it.
pub async fn handle(
    reader: &crate::web_player::Reader,
    _account: Option<&str>,
    request: &ApiRequest,
) -> Option<ApiResponse> {
    if let Some(response) = local_answer(request) {
        return Some(response);
    }
    let read = |query: Value| {
        let pending = reader.query(&query);
        async move { free::answer(pending.await) }
    };
    Some(match request {
        ApiRequest::RelatedArtists { id } => ApiResponse::RelatedArtists {
            id: id.clone(),
            result: read(free::artist_query(&format!("spotify:artist:{id}")))
                .await
                .and_then(|body| related_artists(&body)),
        },
        ApiRequest::AlbumQueueTracks {
            id,
            offset,
            request,
        } => ApiResponse::AlbumQueueTracks {
            offset: *offset,
            request: *request,
            result: read(free::album_query(
                &format!("spotify:album:{id}"),
                *offset,
                50,
            ))
            .await
            .and_then(|body| free::album(&body, *offset))
            .map(|album| album.tracks.unwrap_or_default()),
        },
        ApiRequest::SearchPlaylists { query, serial } => ApiResponse::SearchPlaylists {
            query: query.clone(),
            serial: *serial,
            result: read(free::search_query(SearchKind::Playlists, query, 20))
                .await
                .map(|body| playlists_found(&body)),
        },
        ApiRequest::PlaySearch {
            kind,
            query,
            serial,
        } => ApiResponse::PlaySearch {
            query: query.clone(),
            serial: *serial,
            result: read(free::search_query(search_kind(kind), query, 1))
                .await
                .map(|body| best_match(search_kind(kind), &body)),
        },
        _ => return None,
    })
}

/// The player requests a Free sign-in answers on the spot. Spotify takes
/// no remote commands from a Free account and its Connect endpoints refuse
/// the token, while the helper reports the account's playback itself
/// (`LocalState`): there is no other device to read or command. The app
/// asks for the playback state once at sign-in, before the helper reports
/// ready, so the answer has to come from here rather than from the app not
/// asking.
pub fn local_answer(request: &ApiRequest) -> Option<ApiResponse> {
    let _ = request;
    None
}

/// The artists Spotify lists as related, from the artist's page.
pub fn related_artists(body: &Value) -> Result<Vec<Artist>, ApiError> {
    let _ = body;
    Err(ApiError::Decode("not read yet".into()))
}

/// The playlists a playlist search found.
pub fn playlists_found(body: &Value) -> Page<Playlist> {
    let _ = body;
    Page::default()
}

fn search_kind(kind: &str) -> SearchKind {
    match kind {
        "album" => SearchKind::Albums,
        "artist" => SearchKind::Artists,
        "playlist" => SearchKind::Playlists,
        _ => SearchKind::Tracks,
    }
}

/// The URI of the first thing a search found: what `play-search` plays.
pub fn best_match(kind: SearchKind, body: &Value) -> Option<String> {
    let _ = (kind, body);
    None
}

#[allow(dead_code)]
fn first_track(page: Option<Page<Track>>) -> Option<String> {
    page?.items.into_iter().next().map(|track| track.uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::RemoteAction;

    fn body(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    const RELATED: &str = include_str!("testdata/free/related-artists.json");
    const SEARCH_PLAYLISTS: &str = include_str!("testdata/free/search-playlists.json");
    const SEARCH_TRACKS: &str = include_str!("testdata/free/search-tracks.json");
    const ALBUM: &str = include_str!("testdata/free/album.json");

    /// An artist's page lists the artists Spotify relates to them.
    #[test]
    fn related_artists_come_from_the_artists_page() {
        let artists = related_artists(&body(RELATED)).unwrap();
        assert_eq!(artists.len(), 3);
        assert_eq!(artists[0].name, "Nile Rodgers");
        assert_eq!(artists[0].id, "3yDIp0kaq9EFKe07X1X2rz");
        assert_eq!(artists[0].uri, "spotify:artist:3yDIp0kaq9EFKe07X1X2rz");
        assert_eq!(artists[0].images.len(), 3);
    }

    /// A page with no related artists says so instead of failing.
    #[test]
    fn an_artist_without_related_artists_has_none() {
        let page = serde_json::json!({"data": {"artistUnion": {"uri": "spotify:artist:a"}}});
        assert_eq!(related_artists(&page).unwrap(), Vec::new());
        assert!(related_artists(&serde_json::json!({"data": null})).is_err());
    }

    /// A playlist search gives the playlists found and how many there are.
    #[test]
    fn a_playlist_search_lists_the_playlists_found() {
        let page = playlists_found(&body(SEARCH_PLAYLISTS));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 195);
        assert_eq!(page.items[0].name, "Daft Punk Greatest Hits");
        assert_eq!(page.items[0].uri, "spotify:playlist:2jTy5QwqWJ1ZUv2XeJPYbn");
        assert_eq!(page.items[0].owner.id.as_deref(), Some("owner1"));
    }

    /// `play-search` plays the first song, or playlist, found.
    #[test]
    fn play_search_plays_the_first_match() {
        assert_eq!(
            best_match(SearchKind::Tracks, &body(SEARCH_TRACKS)).as_deref(),
            Some("spotify:track:2cGxRwrMyEAp8dEbuZaVv6")
        );
        assert_eq!(
            best_match(SearchKind::Playlists, &body(SEARCH_PLAYLISTS)).as_deref(),
            Some("spotify:playlist:2jTy5QwqWJ1ZUv2XeJPYbn")
        );
        assert_eq!(
            best_match(SearchKind::Albums, &body(SEARCH_TRACKS)),
            None,
            "no albums in a song search"
        );
    }

    /// Queueing an album reads its songs from the album page.
    #[test]
    fn an_album_queues_the_songs_on_its_page() {
        let songs = free::album(&body(ALBUM), 0).unwrap().tracks.unwrap();
        assert!(!songs.items.is_empty());
        assert!(songs.items[0].uri.starts_with("spotify:track:"));
    }

    /// The player's own requests never reach the Web API for a Free
    /// sign-in: nothing plays elsewhere, there is no other device, and a
    /// remote command is refused rather than sent.
    #[test]
    fn a_free_sign_in_answers_player_requests_itself() {
        match local_answer(&ApiRequest::PlaybackState { seq: 7 }) {
            Some(ApiResponse::PlaybackState {
                seq: 7,
                result: Ok(None),
            }) => {}
            other => panic!("playback state: {other:?}"),
        }
        match local_answer(&ApiRequest::Devices) {
            Some(ApiResponse::Devices(Ok(devices))) => assert!(devices.is_empty()),
            other => panic!("devices: {other:?}"),
        }
        match local_answer(&ApiRequest::Queue { seq: 3 }) {
            Some(ApiResponse::Queue { seq: 3, result }) => assert!(result.is_err()),
            other => panic!("queue: {other:?}"),
        }
        let remote = ApiRequest::Remote {
            action: RemoteAction::Next,
            device_id: None,
            play: None,
            position_ms: 0,
            percent: 0,
            flag: false,
            repeat: String::new(),
        };
        match local_answer(&remote) {
            Some(ApiResponse::Remote {
                action: RemoteAction::Next,
                result: Err(_),
            }) => {}
            other => panic!("remote: {other:?}"),
        }
        assert!(local_answer(&ApiRequest::Me).is_none());
    }
}
