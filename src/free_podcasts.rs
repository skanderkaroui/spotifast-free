//! Podcast reads for a Spotify Free account, through Spotify's own web
//! player (see `free_reads`): a podcast, its episodes, one episode, and the
//! account's saved podcasts and Your Episodes. Read-only: nothing here marks
//! an episode played or saves anything.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::api::models::{Episode, Page, Show};

fn persisted(operation: &str, hash: &str, variables: Value) -> Value {
    json!({
        "operationName": operation,
        "variables": variables,
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}},
    })
}

/// A podcast's page: name, publisher, description and cover.
pub fn show_query(uri: &str) -> Value {
    persisted(
        "queryShowMetadataV2",
        "b475447846f37cc426add800b995a7859c169c53c72b62de6338c0c40a5dacea",
        json!({"uri": uri, "includeContentCapabilityTrait": true, "includeEpisodeContentRatingsV2": true}),
    )
}

/// A page of a podcast's episodes, newest first.
pub fn show_episodes_query(uri: &str, offset: u32, limit: u32) -> Value {
    persisted(
        "queryPodcastEpisodes",
        "3539d746cf882f3909660de40b4ef472b3f5893a0761d617c282541de5d412c5",
        json!({"uri": uri, "offset": offset, "limit": limit, "includeEpisodeContentRatingsV2": true}),
    )
}

/// One episode, with the podcast it belongs to.
pub fn episode_query(uri: &str) -> Value {
    persisted(
        "getEpisodeOrChapter",
        "5f77db47e5a2ce6680330bb61317aa09e17ddd1d189513cb218e7cd3e43288be",
        json!({"uri": uri, "includeEpisodeContentRatingsV2": true}),
    )
}

/// A podcast from its page.
pub fn show(_body: &Value) -> Result<Show, ApiError> {
    Err(ApiError::Decode("not read yet".into()))
}

/// A page of a podcast's episodes.
pub fn show_episodes(_body: &Value, _offset: u32) -> Result<Page<Episode>, ApiError> {
    Err(ApiError::Decode("not read yet".into()))
}

/// One episode from its own answer.
pub fn episode(_body: &Value) -> Result<Episode, ApiError> {
    Err(ApiError::Decode("not read yet".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// A podcast's page gives its name, publisher, description and cover.
    #[test]
    fn a_podcast_reads_with_its_publisher_and_cover() {
        let show = show(&body(include_str!("testdata/free/show.json"))).unwrap();
        assert_eq!(show.id, "4rOoJ6Egrf8K2IrywzwOMk");
        assert_eq!(show.uri, "spotify:show:4rOoJ6Egrf8K2IrywzwOMk");
        assert_eq!(show.name, "The Joe Rogan Experience");
        assert_eq!(show.publisher, "Joe Rogan");
        assert!(show.description.starts_with("The official podcast"));
        assert_eq!(show.images.len(), 3);
        assert_eq!(
            show.external_urls.spotify.as_deref(),
            Some("https://open.spotify.com/show/4rOoJ6Egrf8K2IrywzwOMk")
        );
    }

    /// A page of episodes, newest first, each with its length, date and
    /// podcast, and where the next page starts.
    #[test]
    fn a_podcast_lists_its_episodes() {
        let page =
            show_episodes(&body(include_str!("testdata/free/show-episodes.json")), 0).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 2770);
        assert_eq!(page.next.as_deref(), Some("offset:2"));
        let first = &page.items[0];
        assert_eq!(first.id, "2rKMiQw2A58GL5ZHH8jfHo");
        assert_eq!(first.uri, "spotify:episode:2rKMiQw2A58GL5ZHH8jfHo");
        assert_eq!(first.name, "#2570 - Mystery Boys");
        assert_eq!(first.duration_ms, 9_821_140);
        assert_eq!(first.release_date.as_deref(), Some("2026-10-23"));
        assert!(first.explicit);
        assert_eq!(first.images.len(), 3);
        assert_eq!(first.resume_ms(), None, "not started");
        assert_eq!(
            first.show.as_ref().map(|show| show.name.as_str()),
            Some("The Joe Rogan Experience")
        );
    }

    /// One episode, asked for by its link, names its podcast.
    #[test]
    fn an_episode_reads_with_its_podcast() {
        let episode = episode(&body(include_str!("testdata/free/episode.json"))).unwrap();
        assert_eq!(episode.id, "2rKMiQw2A58GL5ZHH8jfHo");
        assert_eq!(episode.duration_ms, 9_821_140);
        assert!(!episode.description.is_empty());
        let show = episode.show.unwrap();
        assert_eq!(show.id, "4rOoJ6Egrf8K2IrywzwOMk");
        assert_eq!(show.publisher, "", "the episode's answer has no publisher");
    }

    /// Where an episode was left: started, finished, or not begun.
    #[test]
    fn an_episodes_place_reads_as_the_web_api_resume_point() {
        let mut data = body(include_str!("testdata/free/episode.json"));
        data["data"]["episodeUnionV2"]["playedState"] =
            json!({"playPositionMilliseconds": 60000, "state": "IN_PROGRESS"});
        assert_eq!(episode(&data).unwrap().resume_ms(), Some(60_000));
        data["data"]["episodeUnionV2"]["playedState"] =
            json!({"playPositionMilliseconds": 0, "state": "COMPLETED"});
        let done = episode(&data).unwrap();
        assert!(done.resume_point.unwrap().fully_played);
    }
}
