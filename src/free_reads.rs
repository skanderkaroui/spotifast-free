//! Reads for a Spotify Free account, through Spotify's own web player.
//!
//! Spotify closed the shared Web API app to new sign-ins, and lets only
//! Premium accounts make an app of their own. A Free account signs in on
//! Spotify's own page in the playback helper instead, and its reads go
//! through the web player's own API (`pathfinder`), which the page makes
//! with its own sign-in (`web_player::WebPlayer::query`). Spotify answers
//! the official Web API for that sign-in with "rate limit exceeded" from the
//! first request (tried 2026-10-26), so these are the web player's queries,
//! shaped here as the Web API answers them.
//!
//! The queries are the web player's persisted queries: an operation name
//! and the hash of its query text, recorded from open.spotify.com. Spotify
//! changes a hash when it changes the query; the answer is then an error,
//! never another account's data.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::api::models::{
    Album, ArtistRef, Image, Owner, Page, PlayableItem, Playlist, PlaylistItem, SavedTrack, Track,
    TrackCount, User,
};

fn persisted(operation: &str, hash: &str, variables: Value) -> Value {
    json!({
        "operationName": operation,
        "variables": variables,
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}},
    })
}

/// The signed-in account's name, username and avatar.
pub fn profile_query() -> Value {
    persisted(
        "profileAttributes",
        "08ffb4730af3746e04a8301396f20875dbbce10c75243803091a9274eacc8ac0",
        json!({}),
    )
}

/// The signed-in account's plan and country.
pub fn account_query() -> Value {
    persisted(
        "accountAttributes",
        "d48062d0df0a256ff78ed9e232b83d238dcd710b371efd304b0e178b62595582",
        json!({}),
    )
}

/// The account's saved songs and albums, newest first, as one page of the
/// collection service. The page sends it to `collection/v2/paging`.
pub fn collection_query(username: &str, limit: u32) -> Value {
    json!({"collection": {"username": username, "set": "collection", "limit": limit}})
}

/// The songs among the collection's rows, newest first, with when each was
/// saved (seconds since the epoch).
pub fn liked_uris(body: &Value) -> Vec<(String, Option<String>)> {
    body.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let uri = str_at(row, "/uri")?;
            uri.starts_with("spotify:track:").then(|| {
                let added = str_at(row, "/added_at")
                    .and_then(|seconds| seconds.parse::<i64>().ok())
                    .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
                    .map(|at| at.to_string());
                (uri.to_owned(), added)
            })
        })
        .collect()
}

/// The details of up to 50 songs, by URI.
pub fn tracks_query(uris: &[String]) -> Value {
    persisted(
        "decorateContextTracks",
        "383de00240775c39a6afe0b1055dc562b2a3930894201f9762f3fc32a74971c7",
        json!({"uris": uris}),
    )
}

/// A page of Liked Songs: the saved songs at `offset`, with their details.
pub fn saved_page(
    saved: &[(String, Option<String>)],
    details: &Value,
    offset: u32,
) -> Page<SavedTrack> {
    let songs: std::collections::HashMap<String, Track> = details
        .pointer("/data/tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|song| track(song, None))
        .map(|song| (song.uri.clone(), song))
        .collect();
    let total = u32::try_from(saved.len()).unwrap_or(u32::MAX);
    let start = (offset as usize).min(saved.len());
    let rows = &saved[start..(start + 50).min(saved.len())];
    let items = rows
        .iter()
        .filter_map(|(uri, added)| {
            Some(SavedTrack {
                added_at: added.clone(),
                track: songs.get(uri)?.clone(),
            })
        })
        .collect();
    Page {
        items,
        total,
        limit: 50,
        offset,
        next: next(offset, rows.len(), total),
    }
}

/// One page of the account's library: playlists, albums and the Liked Songs
/// collection, newest first, as the web player's sidebar lists them.
pub fn library_query(offset: u32, limit: u32) -> Value {
    persisted(
        "libraryV3",
        "390c78e5b951029bad359785e69b07b536a509c581cbcd0aded5e5067f187455",
        json!({
            "filters": [], "order": null, "textFilter": "",
            "features": ["LIKED_SONGS", "YOUR_EPISODES_V2", "PRERELEASES", "PRERELEASES_V2", "CLIPS", "EVENTS"],
            "limit": limit, "offset": offset, "flatten": true, "expandedFolders": [],
            "folderUri": null, "includeFoldersWhenFlattening": false,
        }),
    )
}

/// One page of a playlist.
pub fn playlist_query(uri: &str, offset: u32, limit: u32) -> Value {
    persisted(
        "fetchPlaylist",
        "21ea11a6ac75a5f3e01fd365d29c18e0a4a8b7dc09f89bbb5d73910190c6acaf",
        json!({
            "uri": uri, "offset": offset, "limit": limit,
            "enableWatchFeedEntrypoint": false, "includeEpisodeContentRatingsV2": true,
        }),
    )
}

/// An album and one page of its songs.
pub fn album_query(uri: &str, offset: u32, limit: u32) -> Value {
    persisted(
        "getAlbum",
        "0b6ce3511e1b02e4a40fa95440f992acbe85080eb308af9194322d708fc1914f",
        json!({"uri": uri, "locale": "", "offset": offset, "limit": limit}),
    )
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn u32_at(value: &Value, pointer: &str) -> Option<u32> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// The id at the end of a `spotify:kind:id` URI.
fn uri_id(uri: &str) -> Option<String> {
    uri.rsplit(':')
        .next()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Images from the web player's `{sources: [{url, width, height}]}`.
fn images(value: Option<&Value>) -> Vec<Image> {
    value
        .and_then(|value| value.get("sources"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            Some(Image {
                url: str_at(source, "/url")?.to_owned(),
                width: u32_at(source, "/width"),
                height: u32_at(source, "/height"),
            })
        })
        .collect()
}

fn artists(value: Option<&Value>) -> Vec<ArtistRef> {
    value
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|artist| {
            let uri = str_at(artist, "/uri")?;
            Some(ArtistRef {
                id: uri_id(uri),
                name: str_at(artist, "/profile/name")
                    .unwrap_or_default()
                    .to_owned(),
                uri: Some(uri.to_owned()),
            })
        })
        .collect()
}

/// A song as the web player gives it, in a playlist or an album.
fn track(data: &Value, album: Option<&Album>) -> Option<Track> {
    let uri = str_at(data, "/uri")?;
    if !uri.starts_with("spotify:track:") {
        return None;
    }
    let album = album.cloned().or_else(|| {
        let album = data.get("albumOfTrack")?;
        let uri = str_at(album, "/uri")?;
        Some(Album {
            id: uri_id(uri).unwrap_or_default(),
            uri: uri.to_owned(),
            name: str_at(album, "/name").unwrap_or_default().to_owned(),
            images: images(album.get("coverArt")),
            artists: artists(album.get("artists")),
            ..Album::default()
        })
    });
    Some(Track {
        id: uri_id(uri),
        name: str_at(data, "/name").unwrap_or_default().to_owned(),
        uri: uri.to_owned(),
        duration_ms: u32_at(data, "/trackDuration/totalMilliseconds")
            .or_else(|| u32_at(data, "/duration/totalMilliseconds"))
            .unwrap_or_default(),
        explicit: str_at(data, "/contentRating/label") == Some("EXPLICIT"),
        artists: artists(data.get("artists")),
        album,
        track_number: u32_at(data, "/trackNumber"),
        disc_number: u32_at(data, "/discNumber"),
        is_playable: data
            .pointer("/playability/playable")
            .and_then(Value::as_bool),
        ..Track::default()
    })
}

/// A playlist's header, from a library row or the playlist's own answer.
fn playlist_header(data: &Value) -> Option<Playlist> {
    let uri = str_at(data, "/uri")?;
    let owner = data.pointer("/ownerV2/data");
    Some(Playlist {
        id: uri_id(uri)?,
        uri: uri.to_owned(),
        name: str_at(data, "/name").unwrap_or_default().to_owned(),
        description: str_at(data, "/description").map(str::to_owned),
        images: data
            .pointer("/images/items/0")
            .map(|image| images(Some(image)))
            .unwrap_or_default(),
        owner: Owner {
            id: owner
                .and_then(|owner| str_at(owner, "/username"))
                .map(str::to_owned),
            display_name: owner
                .and_then(|owner| str_at(owner, "/name"))
                .map(str::to_owned),
            uri: owner
                .and_then(|owner| str_at(owner, "/uri"))
                .map(str::to_owned),
        },
        tracks: u32_at(data, "/content/totalCount").map(|total| TrackCount { total }),
        ..Playlist::default()
    })
}

/// A page of the account's playlists, from a page of its library. Albums,
/// folders and Liked Songs are left out: the app lists those elsewhere.
pub fn library_playlists(body: &Value, offset: u32) -> Result<Page<Playlist>, ApiError> {
    let library = body
        .pointer("/data/me/libraryV3")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Decode("Spotify sent no library".into()))?;
    let rows = library
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let items: Vec<Playlist> = rows
        .iter()
        .filter(|row| {
            row.pointer("/item/data/__typename").and_then(Value::as_str) == Some("Playlist")
        })
        .filter_map(|row| playlist_header(row.pointer("/item/data")?))
        .collect();
    let total = u32_at(library, "/totalCount").unwrap_or_default();
    let fetched = offset.saturating_add(u32::try_from(rows.len()).unwrap_or(u32::MAX));
    Ok(Page {
        total,
        limit: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        offset,
        next: (fetched < total && !rows.is_empty()).then(|| format!("libraryV3:{fetched}")),
        items,
    })
}

/// A playlist's header from its own answer.
pub fn playlist(body: &Value) -> Result<Playlist, ApiError> {
    body.pointer("/data/playlistV2")
        .filter(|value| value.get("uri").is_some())
        .and_then(playlist_header)
        .ok_or_else(|| ApiError::Decode("Spotify sent no playlist".into()))
}

/// The rows of a page of a playlist or of Liked Songs.
fn rows(body: &Value) -> Result<(&[Value], u32), ApiError> {
    let content = body
        .pointer("/data/playlistV2/content")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Decode("Spotify sent no songs".into()))?;
    let items = content
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok((items, u32_at(content, "/totalCount").unwrap_or_default()))
}

fn next(offset: u32, count: usize, total: u32) -> Option<String> {
    let fetched = offset.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
    (fetched < total && count > 0).then(|| format!("offset:{fetched}"))
}

/// A page of a playlist's songs. Episodes and local files the web player
/// lists without a song are left out.
pub fn playlist_items(body: &Value, offset: u32) -> Result<Page<PlaylistItem>, ApiError> {
    let (rows, total) = rows(body)?;
    let items = rows
        .iter()
        .filter_map(|row| {
            let track = track(row.pointer("/itemV2/data")?, None)?;
            Some(PlaylistItem {
                added_at: str_at(row, "/addedAt/isoString").map(str::to_owned),
                item: Some(PlayableItem::Track(track)),
                ..PlaylistItem::default()
            })
        })
        .collect();
    Ok(Page {
        items,
        total,
        limit: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        offset,
        next: next(offset, rows.len(), total),
    })
}

/// An album with the page of its songs the query asked for.
pub fn album(body: &Value, offset: u32) -> Result<Album, ApiError> {
    let data = body
        .pointer("/data/albumUnion")
        .filter(|value| value.get("uri").is_some())
        .ok_or_else(|| ApiError::Decode("Spotify sent no album".into()))?;
    let uri = str_at(data, "/uri").unwrap_or_default();
    let mut album = Album {
        id: uri_id(uri).unwrap_or_default(),
        uri: uri.to_owned(),
        name: str_at(data, "/name").unwrap_or_default().to_owned(),
        album_type: str_at(data, "/type").map(str::to_ascii_lowercase),
        images: images(data.get("coverArt")),
        artists: artists(data.get("artists")),
        release_date: str_at(data, "/date/isoString").map(|date| date.chars().take(10).collect()),
        label: str_at(data, "/label").map(str::to_owned),
        ..Album::default()
    };
    let rows = data
        .pointer("/tracksV2/items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let total = u32_at(data, "/tracksV2/totalCount").unwrap_or_default();
    let header = Album {
        tracks: None,
        ..album.clone()
    };
    let songs: Vec<Track> = rows
        .iter()
        .filter_map(|row| track(row.get("track")?, Some(&header)))
        .collect();
    album.total_tracks = Some(total);
    album.tracks = Some(Page {
        items: songs,
        total,
        limit: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        offset,
        next: next(offset, rows.len(), total),
    });
    Ok(album)
}

/// The body of a successful answer, or why there is none.
pub fn answer(reply: crate::web_player::QueryReply) -> Result<Value, ApiError> {
    use crate::web_player::QueryReply;
    match reply {
        QueryReply::Answered { status, body } if (200..300).contains(&status) => {
            let value: Value =
                serde_json::from_str(&body).map_err(|error| ApiError::Decode(error.to_string()))?;
            if value.get("data").is_none_or(Value::is_null)
                && let Some(message) = value.pointer("/errors/0/message").and_then(Value::as_str)
            {
                return Err(ApiError::Decode(message.to_owned()));
            }
            Ok(value)
        }
        QueryReply::Answered { status: 401, .. } | QueryReply::SignedOut => {
            Err(ApiError::NotSignedIn)
        }
        QueryReply::Answered { status: 429, .. } => Err(ApiError::RateLimited),
        QueryReply::Answered { status, body } => Err(ApiError::Status {
            status,
            message: body.chars().take(200).collect(),
        }),
        QueryReply::Unavailable => Err(ApiError::Network(
            "Spotify's web player did not answer".into(),
        )),
    }
}

/// The account, as the Web API's `/me` gives it, from the web player's
/// profile and account answers.
pub fn user(profile: &Value, account: &Value) -> Result<User, ApiError> {
    let profile = profile
        .pointer("/data/me/profile")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Decode("Spotify sent no profile".into()))?;
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let id = text(profile, "username")
        .ok_or_else(|| ApiError::Decode("Spotify sent no username".into()))?;
    let images = profile
        .pointer("/avatar/sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            Some(Image {
                url: text(source, "url")?,
                width: source
                    .get("width")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok()),
                height: source
                    .get("height")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok()),
            })
        })
        .collect();
    let account = account.pointer("/data/me/account");
    Ok(User {
        uri: text(profile, "uri").or_else(|| Some(format!("spotify:user:{id}"))),
        display_name: text(profile, "name"),
        images,
        // The Web API's own words for the plan.
        product: account
            .and_then(|account| text(account, "product"))
            .map(|product| product.to_ascii_lowercase()),
        country: account.and_then(|account| text(account, "country")),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_player::QueryReply;

    /// The web player's answers, as recorded from open.spotify.com for a
    /// Free account, with the account's own values replaced.
    const PROFILE: &str = include_str!("testdata/free/profile-attributes.json");
    const ACCOUNT: &str = include_str!("testdata/free/account-attributes.json");

    #[test]
    fn a_free_account_reads_as_the_web_api_shapes_it() {
        let user = user(
            &serde_json::from_str(PROFILE).unwrap(),
            &serde_json::from_str(ACCOUNT).unwrap(),
        )
        .unwrap();
        assert_eq!(user.id, "listener1");
        assert_eq!(user.name(), "Listener One");
        assert_eq!(user.product.as_deref(), Some("free"));
        assert_eq!(user.country.as_deref(), Some("SE"));
        assert_eq!(user.uri.as_deref(), Some("spotify:user:listener1"));
        assert_eq!(user.images.len(), 2);
        assert_eq!(user.images[1].width, Some(300));
    }

    const LIBRARY: &str = include_str!("testdata/free/library.json");
    const PLAYLIST: &str = include_str!("testdata/free/playlist.json");
    const ALBUM: &str = include_str!("testdata/free/album.json");

    fn body(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The library lists the account's playlists, without Liked Songs or
    /// albums, and says there is more past this page.
    #[test]
    fn the_library_lists_the_accounts_playlists() {
        let page = library_playlists(&body(LIBRARY), 0).unwrap();
        assert_eq!(page.items.len(), 5, "Liked Songs is not a playlist row");
        assert_eq!(page.total, 54);
        assert!(page.next.is_some(), "54 rows, 6 read");
        let first = &page.items[0];
        assert!(first.uri.starts_with("spotify:playlist:"));
        assert_eq!(first.id, first.uri.rsplit(':').next().unwrap());
        assert_eq!(first.owner.id.as_deref(), Some("listener1"));
        assert_eq!(first.owner.display_name.as_deref(), Some("Listener One"));
        assert_eq!(first.images.len(), 1);
    }

    /// A playlist page gives its songs with their album, artists and
    /// length, and where the next page starts.
    #[test]
    fn a_playlist_page_gives_its_songs() {
        let page = playlist_items(&body(PLAYLIST), 0).unwrap();
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.total, 463);
        assert_eq!(page.next.as_deref(), Some("offset:3"));
        let Some(PlayableItem::Track(song)) = &page.items[0].item else {
            panic!("a song");
        };
        assert_eq!(song.uri, "spotify:track:3GWeTr5MQe85ybRi9nmCIc");
        assert_eq!(song.id.as_deref(), Some("3GWeTr5MQe85ybRi9nmCIc"));
        assert_eq!(song.duration_ms, 94_513);
        assert_eq!(song.track_number, Some(2));
        assert_eq!(song.artists[0].name, "Artist 1");
        let album = song.album.as_ref().unwrap();
        assert_eq!(album.name, "Album 1");
        assert_eq!(album.images.len(), 1);
        assert!(page.items[0].added_at.is_some());
        let header = playlist(&body(PLAYLIST)).unwrap();
        assert_eq!(header.name, "Playlist 1");
        assert_eq!(header.tracks.map(|count| count.total), Some(463));
    }

    /// Liked Songs: the saved songs in the order saved, each with its
    /// details, and a page past the end empty.
    #[test]
    fn liked_songs_page_through_the_saved_songs() {
        let collection = json!({"items": [
            {"uri": "spotify:track:a", "added_at": "1700000000"},
            {"uri": "spotify:album:x", "added_at": "1700000001"},
            {"uri": "spotify:track:b", "added_at": "1600000000"},
        ]});
        let saved = liked_uris(&collection);
        assert_eq!(saved.len(), 2, "albums are not songs");
        assert_eq!(saved[0].1.as_deref(), Some("2023-11-14T22:13:20Z"));
        let details = json!({"data": {"tracks": [
            {"uri": "spotify:track:b", "name": "B", "duration": {"totalMilliseconds": 2000}},
            {"uri": "spotify:track:a", "name": "A", "duration": {"totalMilliseconds": 1000}},
        ]}});
        let page = saved_page(&saved, &details, 0);
        assert_eq!(page.total, 2);
        assert_eq!(page.next, None);
        let names: Vec<_> = page
            .items
            .iter()
            .map(|item| item.track.name.as_str())
            .collect();
        assert_eq!(names, ["A", "B"], "in the order saved");
        assert_eq!(page.items[1].track.duration_ms, 2000);
        assert!(saved_page(&saved, &json!(null), 50).items.is_empty());
    }

    /// An album gives its header and its songs, each pointing back at it.
    #[test]
    fn an_album_gives_its_songs() {
        let album = album(&body(ALBUM), 0).unwrap();
        assert_eq!(album.name, "Random Access Memories");
        assert_eq!(album.album_type.as_deref(), Some("album"));
        assert_eq!(album.release_date.as_deref(), Some("2013-05-20"));
        assert_eq!(album.total_tracks, Some(13));
        let songs = album.tracks.as_ref().unwrap();
        assert_eq!(songs.items.len(), 2);
        assert_eq!(songs.next.as_deref(), Some("offset:2"));
        assert_eq!(songs.items[0].name, "Give Life Back to Music");
        assert_eq!(songs.items[0].track_number, Some(1));
        assert_eq!(
            songs.items[0]
                .album
                .as_ref()
                .map(|album| album.uri.as_str()),
            Some(album.uri.as_str())
        );
    }

    #[test]
    fn a_profile_without_a_username_is_not_an_account() {
        let profile = json!({"data": {"me": {"profile": {"name": "Nobody"}}}});
        assert!(user(&profile, &json!({})).is_err());
        assert!(user(&json!({"data": null}), &json!({})).is_err());
    }

    #[test]
    fn spotify_refusals_keep_their_meaning() {
        let answered = |status: u16, body: &str| QueryReply::Answered {
            status,
            body: body.into(),
        };
        assert!(answer(answered(200, PROFILE)).is_ok());
        assert!(matches!(
            answer(answered(401, "")),
            Err(ApiError::NotSignedIn)
        ));
        assert!(matches!(
            answer(QueryReply::SignedOut),
            Err(ApiError::NotSignedIn)
        ));
        assert!(matches!(
            answer(answered(429, "")),
            Err(ApiError::RateLimited)
        ));
        assert!(matches!(
            answer(answered(403, "Client/request not allowed")),
            Err(ApiError::Status { status: 403, .. })
        ));
        assert!(matches!(
            answer(QueryReply::Unavailable),
            Err(ApiError::Network(_))
        ));
        // A query Spotify no longer knows answers 200 with only errors.
        assert!(matches!(
            answer(answered(
                200,
                r#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#
            )),
            Err(ApiError::Decode(message)) if message == "PersistedQueryNotFound"
        ));
    }
}

/// Live, against Spotify, through the real helper and its saved sign-in:
/// `SPOTIFAST_PLAYBACK_HELPER=... cargo test --lib free_reads::live -- --ignored`.
/// It only reads the account; nothing plays.
#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::*;

    #[test]
    #[ignore = "needs the helper signed in to Spotify"]
    fn the_signed_in_account_reads_through_the_helper() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let player = crate::web_player::WebPlayer::start(
            std::sync::Arc::new(|_| {}),
            std::sync::Arc::new(|_| {}),
            None,
        )
        .unwrap();
        let profile = player.query(&profile_query());
        let account = player.query(&account_query());
        let (profile, account) = runtime.block_on(async { tokio::join!(profile, account) });
        let user = user(&answer(profile).unwrap(), &answer(account).unwrap()).unwrap();
        let read = |query: Value| runtime.block_on(player.query(&query));
        let library = library_playlists(&answer(read(library_query(0, 50))).unwrap(), 0).unwrap();
        let saved = liked_uris(&answer(read(collection_query(&user.id, 10_000))).unwrap());
        let first_fifty: Vec<String> = saved.iter().take(50).map(|(uri, _)| uri.clone()).collect();
        let liked = saved_page(
            &saved,
            &answer(read(tracks_query(&first_fifty))).unwrap(),
            0,
        );
        // The first rows can be podcast lists (Your Episodes); take the
        // first that holds songs.
        let (first, songs) = library
            .items
            .iter()
            .take(6)
            .find_map(|playlist| {
                let songs = playlist_items(
                    &answer(read(playlist_query(&playlist.uri, 0, 100))).ok()?,
                    0,
                )
                .ok()?;
                (!songs.items.is_empty()).then_some((playlist, songs))
            })
            .expect("a playlist with songs among the first six");
        eprintln!("first playlist with songs: {}", first.id.len());
        let album = album(
            &answer(read(album_query(
                "spotify:album:4m2880jivSbbyEGAKfITCa",
                0,
                50,
            )))
            .unwrap(),
            0,
        )
        .unwrap();
        player.shutdown();
        assert!(!user.id.is_empty());
        assert!(user.product.is_some());
        assert!(library.total > 0);
        assert_eq!(
            liked.items.len(),
            first_fifty.len(),
            "every liked song has its details"
        );
        assert!(!songs.items.is_empty());
        assert!(album.tracks.is_some_and(|songs| !songs.items.is_empty()));
        eprintln!(
            "web player reads: plan {:?}; {} library rows, {} playlists on page 1; \
             {} liked songs ({} read); first playlist {} songs ({} read); album {}",
            user.product,
            library.total,
            library.items.len(),
            liked.total,
            liked.items.len(),
            songs.total,
            songs.items.len(),
            album.name,
        );
    }
}
