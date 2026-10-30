//! Podcast reads for a Spotify Free account, through Spotify's own web
//! player (see `free_reads`): a podcast, its episodes, one episode, and the
//! account's saved podcasts and Your Episodes. Read-only: nothing here marks
//! an episode played or saves anything.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::api::models::{
    Episode, ExternalUrls, Image, Page, ResumePoint, SavedEpisode, SavedShow, Show,
};
use crate::backend::{ApiRequest, ApiResponse};

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
fn uri_id(uri: &str) -> String {
    uri.rsplit(':').next().unwrap_or_default().to_owned()
}

/// The web player's link to a `spotify:kind:id` URI.
fn link(uri: &str) -> ExternalUrls {
    let mut parts = uri.split(':').skip(1);
    ExternalUrls {
        spotify: match (parts.next(), parts.next()) {
            (Some(kind), Some(id)) => Some(format!("https://open.spotify.com/{kind}/{id}")),
            _ => None,
        },
    }
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

/// A podcast as the web player describes it, on its page or under an
/// episode.
fn podcast(data: &Value) -> Option<Show> {
    let uri = str_at(data, "/uri")?.to_owned();
    Some(Show {
        id: uri_id(&uri),
        name: str_at(data, "/name").unwrap_or_default().to_owned(),
        publisher: str_at(data, "/publisher/name")
            .unwrap_or_default()
            .to_owned(),
        description: str_at(data, "/description").unwrap_or_default().to_owned(),
        images: images(data.get("coverArt")),
        external_urls: link(&uri),
        uri,
        ..Show::default()
    })
}

/// An episode as the web player describes it. Its place is Spotify's own
/// (`playedState`): started, finished, or not begun.
fn episode_data(data: &Value) -> Option<Episode> {
    let uri = str_at(data, "/uri")?.to_owned();
    if !uri.starts_with("spotify:episode:") {
        return None;
    }
    let resume_point = data.get("playedState").and_then(|state| {
        let place = str_at(state, "/state")?;
        Some(ResumePoint {
            fully_played: place == "COMPLETED",
            resume_position_ms: u32_at(state, "/playPositionMilliseconds").unwrap_or_default(),
        })
    });
    Some(Episode {
        id: uri_id(&uri),
        name: str_at(data, "/name").unwrap_or_default().to_owned(),
        duration_ms: u32_at(data, "/duration/totalMilliseconds").unwrap_or_default(),
        description: str_at(data, "/description").unwrap_or_default().to_owned(),
        images: images(data.get("coverArt")),
        release_date: str_at(data, "/releaseDate/isoString")
            .map(|date| date.chars().take(10).collect()),
        explicit: str_at(data, "/contentRating/label") == Some("EXPLICIT"),
        resume_point,
        show: data.pointer("/podcastV2/data").and_then(podcast),
        external_urls: link(&uri),
        uri,
    })
}

fn next(offset: u32, count: usize, total: u32) -> Option<String> {
    let fetched = offset.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
    (fetched < total && count > 0).then(|| format!("offset:{fetched}"))
}

/// Whether a show page answer is an audiobook, which Spotify lists with
/// saved podcasts (REA: `queryShowMetadataV2` answers `podcastUnionV2`
/// with `__typename` "Podcast" or "Audiobook"). `None` when it says neither.
pub fn is_audiobook(body: &Value) -> Option<bool> {
    match str_at(body, "/data/podcastUnionV2/__typename")? {
        "Audiobook" => Some(true),
        "Podcast" => Some(false),
        _ => None,
    }
}

/// A podcast from its page. An audiobook's page has the same shape and is
/// kept too: Spotify lists both among saved shows, and the app sets
/// audiobooks apart (see `is_audiobook`).
pub fn show(body: &Value) -> Result<Show, ApiError> {
    body.pointer("/data/podcastUnionV2")
        .filter(|data| matches!(str_at(data, "/__typename"), Some("Podcast" | "Audiobook")))
        .and_then(podcast)
        .ok_or_else(|| ApiError::Decode("Spotify sent no podcast".into()))
}

/// A page of a podcast's episodes.
pub fn show_episodes(body: &Value, offset: u32) -> Result<Page<Episode>, ApiError> {
    let list = body
        .pointer("/data/podcastUnionV2/episodesV2")
        .filter(|value| value.is_object())
        .ok_or_else(|| ApiError::Decode("Spotify sent no episodes".into()))?;
    let rows = list
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let total = u32_at(list, "/totalCount").unwrap_or_default();
    Ok(Page {
        items: rows
            .iter()
            .filter_map(|row| episode_data(row.pointer("/entity/data")?))
            .collect(),
        total,
        limit: u32::try_from(rows.len()).unwrap_or(u32::MAX),
        offset,
        next: next(offset, rows.len(), total),
    })
}

/// One episode from its own answer.
pub fn episode(body: &Value) -> Result<Episode, ApiError> {
    body.pointer("/data/episodeUnionV2")
        .and_then(episode_data)
        .ok_or_else(|| ApiError::Decode("Spotify sent no episode".into()))
}

/// One of the account's collections, newest first: `show` for its saved
/// podcasts, `listenlater` for Your Episodes. The page sends it to
/// `collection/v2/paging`.
pub fn collection_query(username: &str, set: &str) -> Value {
    json!({"collection": {"username": username, "set": set, "limit": 10_000}})
}

/// The details of up to 50 episodes, by URI.
pub fn episodes_query(uris: &[String]) -> Value {
    persisted(
        "decorateContextEpisodesOrChapters",
        "383de00240775c39a6afe0b1055dc562b2a3930894201f9762f3fc32a74971c7",
        json!({"uris": uris}),
    )
}

/// The collection's rows of one kind (`spotify:show:`), newest first, with
/// when each was saved.
pub fn saved_rows(body: &Value, prefix: &str) -> Vec<(String, Option<String>)> {
    body.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let uri = str_at(row, "/uri")?;
            uri.starts_with(prefix).then(|| {
                let added = str_at(row, "/added_at")
                    .and_then(|seconds| seconds.parse::<i64>().ok())
                    .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
                    .map(|at| at.to_string());
                (uri.to_owned(), added)
            })
        })
        .collect()
}

/// The rows at `offset`, a page of 50.
fn rows_at(saved: &[(String, Option<String>)], offset: u32) -> &[(String, Option<String>)] {
    let start = (offset as usize).min(saved.len());
    &saved[start..(start + PAGE).min(saved.len())]
}

const PAGE: usize = 50;

fn saved_page<T>(items: Vec<T>, saved: usize, rows: usize, offset: u32) -> Page<T> {
    let total = u32::try_from(saved).unwrap_or(u32::MAX);
    Page {
        items,
        total,
        limit: PAGE as u32,
        offset,
        next: next(offset, rows, total),
    }
}

/// A page of Your Episodes: the saved episodes at `offset`, with their
/// details.
pub fn saved_episodes(
    saved: &[(String, Option<String>)],
    details: &Value,
    offset: u32,
) -> Page<SavedEpisode> {
    let episodes: std::collections::HashMap<String, Episode> = details
        .pointer("/data/episodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(episode_data)
        .map(|episode| (episode.uri.clone(), episode))
        .collect();
    let rows = rows_at(saved, offset);
    let items = rows
        .iter()
        .filter_map(|(uri, added)| {
            Some(SavedEpisode {
                added_at: added.clone(),
                episode: episodes.get(uri)?.clone(),
            })
        })
        .collect();
    saved_page(items, saved.len(), rows.len(), offset)
}

/// The saved rows at `offset`, whose details to read.
pub fn saved_uris(saved: &[(String, Option<String>)], offset: u32) -> Vec<String> {
    rows_at(saved, offset)
        .iter()
        .map(|(uri, _)| uri.clone())
        .collect()
}

/// A page of saved podcasts: the rows at `offset`, each with its own page's
/// answer, in the same order.
pub fn saved_shows(
    saved: &[(String, Option<String>)],
    pages: &[Value],
    offset: u32,
) -> Page<SavedShow> {
    let rows = rows_at(saved, offset);
    let items = rows
        .iter()
        .zip(pages)
        .filter_map(|((_, added), page)| {
            Some(SavedShow {
                added_at: added.clone(),
                show: show(page).ok()?,
            })
        })
        .collect();
    saved_page(items, saved.len(), rows.len(), offset)
}

/// The newest episodes Home lists per saved podcast.
const HOME_EPISODES_PER_SHOW: u32 = 5;

/// The podcast reads a Free sign-in answers through the web player, shaped
/// as the Web API's answers. `None` for any other request.
pub async fn handle(
    reader: &crate::web_player::Reader,
    account: Option<&str>,
    request: &ApiRequest,
) -> Option<ApiResponse> {
    let read = |query: Value| {
        let pending = reader.query(&query);
        async move { crate::free_reads::answer(pending.await) }
    };
    let show_uri = |id: &str| format!("spotify:show:{id}");
    Some(match request {
        ApiRequest::SavedEpisodes { offset } => ApiResponse::SavedEpisodes {
            offset: *offset,
            result: async {
                let account = account.ok_or(ApiError::NotSignedIn)?;
                let saved = saved_rows(
                    &read(collection_query(account, "listenlater")).await?,
                    "spotify:episode:",
                );
                let uris = saved_uris(&saved, *offset);
                if uris.is_empty() {
                    return Ok(saved_episodes(&saved, &Value::Null, *offset));
                }
                Ok(saved_episodes(
                    &saved,
                    &read(episodes_query(&uris)).await?,
                    *offset,
                ))
            }
            .await,
        },
        ApiRequest::SavedShows { offset } => ApiResponse::SavedShows {
            offset: *offset,
            result: async {
                let account = account.ok_or(ApiError::NotSignedIn)?;
                let saved = saved_rows(
                    &read(collection_query(account, "show")).await?,
                    "spotify:show:",
                );
                let mut reads = tokio::task::JoinSet::new();
                for (index, uri) in saved_uris(&saved, *offset).into_iter().enumerate() {
                    let answer = read(show_query(&uri));
                    reads.spawn(async move { (index, answer.await) });
                }
                // A page that did not answer stays null: its show is left out.
                let mut pages = vec![Value::Null; reads.len()];
                while let Some(joined) = reads.join_next().await {
                    if let Ok((index, Ok(page))) = joined {
                        pages[index] = page;
                    }
                }
                Ok(saved_shows(&saved, &pages, *offset))
            }
            .await,
        },
        ApiRequest::Show { id } => ApiResponse::Show {
            id: id.clone(),
            result: read(show_query(&show_uri(id)))
                .await
                .and_then(|body| show(&body)),
        },
        ApiRequest::ShowEpisodes { id, offset } => ApiResponse::ShowEpisodes {
            id: id.clone(),
            offset: *offset,
            result: read(show_episodes_query(&show_uri(id), *offset, 50))
                .await
                .and_then(|body| show_episodes(&body, *offset)),
        },
        ApiRequest::Episode { id } => ApiResponse::Episode {
            id: id.clone(),
            result: read(episode_query(&format!("spotify:episode:{id}")))
                .await
                .and_then(|body| episode(&body)),
        },
        ApiRequest::HomeEpisodes { shows, generation } => {
            // The shows are read together; each answers for itself, in the
            // order asked for.
            let mut reads = tokio::task::JoinSet::new();
            for (index, asked) in shows.iter().enumerate() {
                let query = show_episodes_query(&show_uri(&asked.id), 0, HOME_EPISODES_PER_SHOW);
                let answer = read(query);
                reads.spawn(async move { (index, answer.await) });
            }
            let mut answers = Vec::new();
            while let Some(joined) = reads.join_next().await {
                if let Ok(answer) = joined {
                    answers.push(answer);
                }
            }
            answers.sort_by_key(|(index, _)| *index);
            let mut failure = None;
            let mut found = Vec::new();
            for (index, answer) in answers {
                match answer.and_then(|body| show_episodes(&body, 0)) {
                    Ok(page) => found.push((shows[index].clone(), page.items)),
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
            ApiResponse::HomeEpisodes {
                generation: *generation,
                result: match failure {
                    Some(error) if found.is_empty() && !shows.is_empty() => Err(error),
                    _ => Ok(found),
                },
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// The show page says podcast or audiobook; an audiobook still lists
    /// among saved shows so the app can set it apart.
    #[test]
    fn a_show_page_tells_audiobooks_from_podcasts() {
        let podcast = body(include_str!("testdata/free/show.json"));
        assert_eq!(
            is_audiobook(&podcast),
            Some(false),
            "recorded page is a Podcast"
        );
        let mut audiobook = podcast.clone();
        audiobook["data"]["podcastUnionV2"]["__typename"] = "Audiobook".into();
        assert_eq!(is_audiobook(&audiobook), Some(true));
        assert!(show(&audiobook).is_ok(), "audiobooks stay in the list");
        let mut restricted = podcast;
        restricted["data"]["podcastUnionV2"]["__typename"] = "RestrictedContent".into();
        assert_eq!(is_audiobook(&restricted), None);
        assert!(show(&restricted).is_err());
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

    /// Your Episodes: the saved episodes in the order saved, each with its
    /// podcast, and songs or podcasts among the rows left out.
    #[test]
    fn your_episodes_page_through_the_saved_episodes() {
        let collection = json!({"items": [
            {"uri": "spotify:episode:2rKMiQw2A58GL5ZHH8jfHo", "added_at": "1700000000"},
            {"uri": "spotify:track:x", "added_at": "1700000001"},
        ]});
        let saved = saved_rows(&collection, "spotify:episode:");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.as_deref(), Some("2023-11-14T22:13:20Z"));
        let page = saved_episodes(
            &saved,
            &body(include_str!("testdata/free/episodes.json")),
            0,
        );
        assert_eq!(page.total, 1);
        assert_eq!(page.next, None);
        let episode = &page.items[0].episode;
        assert_eq!(episode.name, "#2570 - Mystery Boys");
        assert_eq!(episode.id, "2rKMiQw2A58GL5ZHH8jfHo");
        assert_eq!(episode.duration_ms, 9_821_140);
        let show = episode.show.as_ref().unwrap();
        assert_eq!(show.publisher, "Joe Rogan");
        assert_eq!(
            page.items[0].added_at.as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert!(saved_episodes(&saved, &json!(null), 50).items.is_empty());
    }

    /// Saved podcasts: each saved show read from its own page, in the order
    /// saved; a show whose page did not answer is left out.
    #[test]
    fn saved_podcasts_read_from_their_pages() {
        let collection = json!({"items": [
            {"uri": "spotify:show:4rOoJ6Egrf8K2IrywzwOMk", "added_at": "1700000000"},
            {"uri": "spotify:show:gone", "added_at": "1600000000"},
        ]});
        let saved = saved_rows(&collection, "spotify:show:");
        let page = saved_shows(
            &saved,
            &[body(include_str!("testdata/free/show.json")), json!(null)],
            0,
        );
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].show.name, "The Joe Rogan Experience");
        assert!(page.items[0].added_at.is_some());
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

#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::*;

    /// Each podcast read answers through the helper's web player, as the
    /// backend routes it. Read-only.
    #[test]
    #[ignore = "needs the helper signed in to Spotify"]
    fn podcast_reads_answer_through_the_helper() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let player = crate::web_player::WebPlayer::start(
            std::sync::Arc::new(|_| {}),
            std::sync::Arc::new(|_| {}),
            None,
        )
        .unwrap();
        let reader = player.reader();
        let read = |query: Value| runtime.block_on(reader.query(&query));
        let profile = crate::free_reads::answer(read(crate::free_reads::profile_query())).unwrap();
        let account = crate::free_reads::answer(read(crate::free_reads::account_query())).unwrap();
        let user = crate::free_reads::user(&profile, &account).unwrap();
        let ask = |request: ApiRequest| {
            runtime
                .block_on(handle(&reader, Some(&user.id), &request))
                .expect("a podcast read")
        };
        let rogan = "4rOoJ6Egrf8K2IrywzwOMk".to_string();
        let ApiResponse::Show { result, .. } = ask(ApiRequest::Show { id: rogan.clone() }) else {
            panic!("a podcast")
        };
        let show = result.unwrap();
        let ApiResponse::ShowEpisodes { result, .. } = ask(ApiRequest::ShowEpisodes {
            id: rogan.clone(),
            offset: 50,
        }) else {
            panic!("episodes")
        };
        let episodes = result.unwrap();
        let first = episodes.items[0].id.clone();
        let ApiResponse::Episode { result, .. } = ask(ApiRequest::Episode { id: first }) else {
            panic!("an episode")
        };
        let episode = result.unwrap();
        let ApiResponse::HomeEpisodes { result, .. } = ask(ApiRequest::HomeEpisodes {
            shows: vec![show.clone()],
            generation: 1,
        }) else {
            panic!("Home episodes")
        };
        let home = result.unwrap();
        let ApiResponse::SavedShows { result, .. } = ask(ApiRequest::SavedShows { offset: 0 })
        else {
            panic!("saved podcasts")
        };
        let saved_shows = result.unwrap();
        let ApiResponse::SavedEpisodes { result, .. } =
            ask(ApiRequest::SavedEpisodes { offset: 0 })
        else {
            panic!("Your Episodes")
        };
        let your_episodes = result.unwrap();
        player.shutdown();
        assert_eq!(show.name, "The Joe Rogan Experience");
        assert_eq!(episodes.items.len(), 50);
        assert!(episode.show.is_some());
        assert_eq!(home[0].1.len(), 5);
        assert_eq!(
            your_episodes.items.len(),
            your_episodes.total as usize,
            "every saved episode has its details"
        );
        eprintln!(
            "podcast reads: {} ({} episodes, page 2 read {}); episode {} min; Home {} shows, {} episodes; \
             {} saved podcasts ({} read); {} saved episodes ({} read)",
            show.name,
            episodes.total,
            episodes.items.len(),
            episode.duration_ms / 60_000,
            home.len(),
            home[0].1.len(),
            saved_shows.total,
            saved_shows.items.len(),
            your_episodes.total,
            your_episodes.items.len(),
        );
    }
}
