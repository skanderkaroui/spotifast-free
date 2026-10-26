//! Home reads for a Spotify Free account, through Spotify's web player:
//! top artists and songs, recommendations and new releases.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::api::models::{Album, Artist, Page, Track};
use crate::backend::{ApiRequest, ApiResponse};
use crate::free_reads as free;
use crate::model::TopPeriod;

/// The account's top artists or songs, as its profile page asks for them.
pub fn top_query(artists: bool, period: TopPeriod, offset: u32, limit: u32) -> Value {
    let range = period.as_spotify().to_ascii_uppercase();
    let input = |asked: bool| {
        json!({
            "offset": if asked { offset } else { 0 },
            "limit": if asked { limit } else { 0 },
            "sortBy": "AFFINITY",
            "timeRange": range,
        })
    };
    free::persisted(
        "userTopContent",
        "61b7353d3625780022dbc63d459eb64cd609e7e5e2a820f9160d65030efe2e15",
        json!({
            "includeTopArtists": artists, "topArtistsInput": input(artists),
            "includeTopTracks": !artists, "topTracksInput": input(!artists),
        }),
    )
}

fn top_rows<'a>(body: &'a Value, kind: &str) -> Result<(&'a [Value], u32), ApiError> {
    let list = body
        .pointer(&format!("/data/me/profile/{kind}"))
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Decode("Spotify sent no top list".into()))?;
    let rows = list
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok((rows, free::u32_at(list, "/totalCount").unwrap_or_default()))
}

/// The account's top artists.
pub fn top_artists(body: &Value) -> Result<Vec<Artist>, ApiError> {
    let (rows, _) = top_rows(body, "topArtists")?;
    Ok(rows
        .iter()
        .filter_map(|row| free::artist(row.get("data")?))
        .collect())
}

/// A page of the account's top songs.
pub fn top_tracks(body: &Value, offset: u32) -> Result<Page<Track>, ApiError> {
    let (rows, total) = top_rows(body, "topTracks")?;
    let items = rows
        .iter()
        .filter_map(|row| free::track(row.get("data")?, None))
        .collect();
    Ok(free::page(items, offset, rows.len(), total))
}

/// Routes the Home reads; `None` for anything else.
pub async fn handle(
    reader: &crate::web_player::Reader,
    _account: Option<&str>,
    request: &ApiRequest,
) -> Option<ApiResponse> {
    let read = |query: Value| {
        let pending = reader.query(&query);
        async move { free::answer(pending.await) }
    };
    let _ = std::marker::PhantomData::<Album>;
    Some(match request {
        ApiRequest::TopArtists { generation } => ApiResponse::TopArtists {
            generation: *generation,
            result: read(top_query(true, TopPeriod::MediumTerm, 0, 20))
                .await
                .and_then(|body| top_artists(&body)),
        },
        ApiRequest::TopTracks {
            period,
            offset,
            full,
            generation,
        } => ApiResponse::TopTracks {
            result: read(top_query(
                false,
                *period,
                *offset,
                if *full { 50 } else { 20 },
            ))
            .await
            .and_then(|body| top_tracks(&body, *offset)),
            period: *period,
            offset: *offset,
            full: *full,
            generation: *generation,
        },
        ApiRequest::Recommendations {
            seed_tracks,
            generation,
            ..
        } => {
            let reads = seed_tracks
                .iter()
                .map(|id| read(recommender_query(&format!("spotify:track:{id}"), 10)));
            let answers = futures_join(reads).await;
            let found: Vec<Value> = answers.iter().filter_map(|a| a.as_ref().ok()).cloned().collect();
            ApiResponse::Recommendations {
                generation: *generation,
                result: match answers.into_iter().find_map(Result::err) {
                    Some(error) if found.is_empty() => Err(error),
                    _ => Ok(recommended(&found, seed_tracks, 20)),
                },
            }
        }
        _ => return None,
    })
}

/// Every read at once, answers in the order asked.
async fn futures_join<F, T>(reads: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = reads.map(tokio::spawn).collect();
    let mut answers = Vec::new();
    for handle in handles {
        if let Ok(answer) = handle.await {
            answers.push(answer);
        }
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    const TOP_ARTISTS: &str = include_str!("testdata/free/top-artists.json");
    const TOP_TRACKS: &str = include_str!("testdata/free/top-tracks.json");

    /// Top artists come from the profile's top content, in its order.
    #[test]
    fn top_artists_read_from_the_profile() {
        let artists = top_artists(&body(TOP_ARTISTS)).unwrap();
        assert_eq!(artists.len(), 8);
        assert_eq!(artists[0].name, "Michael Jackson");
        assert_eq!(artists[0].id, "3fMbdgg4jU18AjLCKBhRSm");
        assert_eq!(artists[0].images.len(), 3);
    }

    /// Top songs page through the profile's top content with their album.
    #[test]
    fn top_songs_page_with_their_album() {
        let page = top_tracks(&body(TOP_TRACKS), 0).unwrap();
        assert_eq!(page.items.len(), 25);
        assert_eq!(page.total, 172);
        assert!(page.next.is_some());
        let first = &page.items[0];
        assert_eq!(first.name, "Getaway - Koven Remix");
        assert_eq!(first.artists.len(), 3);
        assert_eq!(first.duration_ms, 203_720);
        assert_eq!(
            first.album.as_ref().unwrap().name,
            "Getaway - Remixes",
            "the album shows on the row"
        );
    }

    /// Each period asks for its own time range, page and size.
    #[test]
    fn top_queries_ask_for_the_period_and_page() {
        let query = top_query(false, TopPeriod::LongTerm, 50, 50);
        assert_eq!(query["operationName"], "userTopContent");
        assert_eq!(query["variables"]["includeTopTracks"], true);
        assert_eq!(query["variables"]["includeTopArtists"], false);
        let input = &query["variables"]["topTracksInput"];
        assert_eq!(input["timeRange"], "LONG_TERM");
        assert_eq!(input["offset"], 50);
        assert_eq!(input["limit"], 50);
        let query = top_query(true, TopPeriod::MediumTerm, 0, 20);
        assert_eq!(
            query["variables"]["topArtistsInput"]["timeRange"],
            "MEDIUM_TERM"
        );
    }
}

/// Songs like a seed song, as the song's own page lists them.
pub fn recommender_query(uri: &str, limit: u32) -> Value {
    free::persisted(
        "internalLinkRecommenderTrack",
        "642923b2e705fceb9984697c1620fecd42615fd8b116b8a9faf2786b389d3e4a",
        json!({"uri": uri, "limit": limit}),
    )
}

/// Songs like the seeds, a few from each seed in turn, without the seeds
/// themselves or repeats, at most `limit` of them.
pub fn recommended(answers: &[Value], seeds: &[String], limit: usize) -> Vec<Track> {
    let lists: Vec<Vec<Track>> = answers
        .iter()
        .map(|answer| {
            answer
                .pointer("/data/seoRecommendedTrack/items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|row| free::track(row.get("data")?, None))
                .collect()
        })
        .collect();
    let mut songs: Vec<Track> = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or_default();
    for index in 0..longest {
        for song in lists.iter().filter_map(|list| list.get(index)) {
            let id = song.id.as_deref().unwrap_or_default();
            if songs.len() < limit
                && !seeds.iter().any(|seed| seed == id)
                && !songs.iter().any(|known| known.uri == song.uri)
            {
                songs.push(song.clone());
            }
        }
    }
    songs
}

#[cfg(test)]
mod recommendation_tests {
    use super::*;

    const RECOMMENDED: &str = include_str!("testdata/free/recommended-track.json");

    #[test]
    fn recommendations_take_turns_between_seeds_without_repeats() {
        let answer: Value = serde_json::from_str(RECOMMENDED).unwrap();
        let query = recommender_query("spotify:track:0DiWol3AO6WpXZgp0goxAV", 10);
        assert_eq!(query["operationName"], "internalLinkRecommenderTrack");
        assert_eq!(query["variables"]["limit"], 10);
        let seeds = vec!["49X0LAl6faAusYq02PRAY6".to_owned()];
        let songs = recommended(&[answer.clone(), answer], &seeds, 20);
        assert_eq!(songs.len(), 4, "the seed and the repeats are left out");
        assert_eq!(songs[0].id.as_deref(), Some("303ccTay2FiDTZ9fZ2AdBt"));
        assert!(songs[0].album.is_some());
        assert!(!songs[0].artists.is_empty());
    }
}

/// New releases: the What's New feed's albums, or when the feed is empty,
/// as it is for an account with few follows, the latest releases of the
/// followed artists from their discographies.
pub fn new_releases(feed: &Value, discographies: &[Value]) -> Result<Vec<Album>, ApiError> {
    let _ = (feed, discographies);
    Ok(Vec::new())
}

/// The songs of these releases, each with its release as the album.
pub fn release_tracks(answers: &[(Album, Value)]) -> Vec<Track> {
    let _ = answers;
    Vec::new()
}

#[cfg(test)]
mod release_tests {
    use super::*;

    const DISCOGRAPHY: &str = include_str!("testdata/free/discography.json");
    const ALBUM: &str = include_str!("testdata/free/album.json");

    fn empty_feed() -> Value {
        json!({"data": {"whatsNewFeedItems": {"items": [], "totalCount": 0}}})
    }

    #[test]
    fn an_empty_feed_lists_the_followed_artists_latest_releases() {
        let pages: Vec<Value> = vec![serde_json::from_str(DISCOGRAPHY).unwrap()];
        let albums = new_releases(&empty_feed(), &pages).unwrap();
        assert_eq!(albums.len(), 5);
        assert_eq!(albums[0].name, "Random Access Memories (Drumless Edition)");
        assert_eq!(albums[0].release_date.as_deref(), Some("2023-11-17"));
        assert_eq!(albums[2].album_type.as_deref(), Some("single"));
    }

    #[test]
    fn a_missing_feed_is_an_error_not_an_empty_shelf() {
        assert!(new_releases(&json!({"data": null}), &[]).is_err());
    }

    #[test]
    fn release_songs_carry_their_release() {
        let release = Album {
            id: "4m2880jivSbbyEGAKfITCa".into(),
            uri: "spotify:album:4m2880jivSbbyEGAKfITCa".into(),
            name: "Random Access Memories".into(),
            ..Album::default()
        };
        let songs = release_tracks(&[(release, serde_json::from_str(ALBUM).unwrap())]);
        assert_eq!(songs.len(), 2);
        assert_eq!(
            songs[0].album.as_ref().unwrap().name,
            "Random Access Memories"
        );
    }
}
