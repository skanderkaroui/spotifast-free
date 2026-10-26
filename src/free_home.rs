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
    let _ = (artists, period, offset, limit);
    Value::Null
}

/// The account's top artists.
pub fn top_artists(body: &Value) -> Result<Vec<Artist>, ApiError> {
    let _ = body;
    Ok(Vec::new())
}

/// A page of the account's top songs.
pub fn top_tracks(body: &Value, offset: u32) -> Result<Page<Track>, ApiError> {
    let _ = (body, offset);
    Ok(Page::default())
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
    let _ = (read, json!(null), std::marker::PhantomData::<Album>);
    let _ = request;
    None
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
