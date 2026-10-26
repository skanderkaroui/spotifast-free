//! Playlist edits for a Spotify Free account, made as Spotify's own web
//! player makes them (see `free_reads`): songs are added, moved and removed
//! with the player's pathfinder mutations, which name rows by their `uid`
//! from `fetchPlaylist`, and a playlist is created, renamed, described and
//! listed on the profile through the playlist service, as JSON changes the
//! page posts with its own sign-in (`playlistChanges` in Diagnostics.js).
//! Each answer has the shape the Web API's answer has for Premium, and the
//! playlist's new revision is reported as its snapshot.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::api::models::{Owner, Playlist};
use crate::backend::{ApiRequest, ApiResponse};
use crate::free_reads as free;
use crate::session_reads::RemovedRow;

/// The web player's playlist mutations share one persisted query.
const MUTATION: &str = "b907ad6b088d5bf7680fa81198996d58398c2d55dfcdf59fcc959e0be98a418a";
/// Rows read per `fetchPlaylist` page when the whole list is needed.
const PAGE: u32 = 100;

/// Answers `request` for a Free sign-in, or `None` when this module does
/// not cover it.
pub async fn handle(
    reader: &crate::web_player::Reader,
    account: Option<&str>,
    request: &ApiRequest,
) -> Option<ApiResponse> {
    Some(match request {
        ApiRequest::AddToPlaylist {
            playlist_id,
            playlist_name,
            uris,
            position,
        } => ApiResponse::PlaylistItemsChanged {
            id: playlist_id.clone(),
            message: format!("Added to {playlist_name}"),
            result: add(reader, playlist_id, uris, *position).await,
        },
        ApiRequest::RemoveFromPlaylist {
            playlist_id, uris, ..
        } => ApiResponse::PlaylistItemsChanged {
            id: playlist_id.clone(),
            message: "Removed from playlist".to_string(),
            result: remove_songs(reader, playlist_id, uris).await,
        },
        ApiRequest::ReorderPlaylist {
            playlist_id,
            range_start,
            insert_before,
            ..
        } => ApiResponse::PlaylistItemsChanged {
            id: playlist_id.clone(),
            message: String::new(),
            result: reorder(reader, playlist_id, *range_start, *insert_before).await,
        },
        ApiRequest::CheckPlaylistDuplicates {
            playlist_id,
            playlist_name,
            items,
            position,
        } => {
            let uris: Vec<String> = items.iter().map(|item| item.uri().to_string()).collect();
            ApiResponse::PlaylistDuplicatesChecked {
                result: rows(reader, playlist_id).await.map(|list| {
                    uris.iter()
                        .filter(|uri| list.rows.iter().any(|row| row.uri == **uri))
                        .cloned()
                        .collect()
                }),
                playlist_id: playlist_id.clone(),
                playlist_name: playlist_name.clone(),
                items: items.clone(),
                position: *position,
            }
        }
        ApiRequest::CreatePlaylist {
            name,
            public,
            description,
        } => {
            ApiResponse::PlaylistCreated(create(reader, account, name, *public, description).await)
        }
        ApiRequest::UpdatePlaylist {
            id,
            name,
            description,
            public,
        } => ApiResponse::PlaylistUpdated {
            id: id.clone(),
            result: update(
                reader,
                account,
                id,
                name.as_deref(),
                description.as_deref(),
                *public,
            )
            .await,
        },
        _ => return None,
    })
}

fn playlist_uri(id: &str) -> String {
    format!("spotify:playlist:{id}")
}

async fn ask(reader: &crate::web_player::Reader, query: Value) -> Result<Value, ApiError> {
    free::answer(reader.query(&query).await)
}

/// One row of a playlist as the web player names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub uid: String,
    pub uri: String,
}

/// Every row of a playlist and the revision they were read at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub revision: Option<String>,
    pub rows: Vec<Row>,
    pub total: u32,
}

/// The rows of one `fetchPlaylist` page, its revision and the list's size.
pub fn page_rows(body: &Value) -> Result<Rows, ApiError> {
    let list = body
        .pointer("/data/playlistV2")
        .filter(|list| list.get("content").is_some())
        .ok_or_else(|| ApiError::Decode("Spotify sent no playlist".into()))?;
    let rows = list
        .pointer("/content/items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| {
            Some(Row {
                uid: row.get("uid")?.as_str()?.to_owned(),
                uri: row
                    .pointer("/itemV2/data/uri")
                    .or_else(|| row.pointer("/itemV2/uri"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect();
    Ok(Rows {
        revision: list
            .get("revisionId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        rows,
        total: free::u32_at(list, "/content/totalCount").unwrap_or_default(),
    })
}

/// Every row of the playlist, page by page from one revision: a list
/// edited between pages is refused rather than read half old, half new.
async fn rows(reader: &crate::web_player::Reader, id: &str) -> Result<Rows, ApiError> {
    let mut all = Rows::default();
    loop {
        let offset = u32::try_from(all.rows.len()).unwrap_or(u32::MAX);
        let page = page_rows(
            &ask(
                reader,
                free::playlist_query(&playlist_uri(id), offset, PAGE),
            )
            .await?,
        )?;
        if offset == 0 {
            all.revision = page.revision;
        } else if page.revision != all.revision {
            return Err(changed());
        }
        all.total = page.total;
        let empty = page.rows.is_empty();
        all.rows.extend(page.rows);
        if empty || all.rows.len() as u32 >= all.total {
            return Ok(all);
        }
    }
}

fn changed() -> ApiError {
    ApiError::Status {
        status: 409,
        message: "The playlist changed meanwhile. Check it and try again.".into(),
    }
}

/// The playlist's revision now, which the app keeps as its snapshot.
async fn revision(
    reader: &crate::web_player::Reader,
    id: &str,
) -> Result<Option<String>, ApiError> {
    Ok(page_rows(&ask(reader, free::playlist_query(&playlist_uri(id), 0, 1)).await?)?.revision)
}

/// Where new or moved rows go, as the web player names it.
fn position(move_type: &str, from_uid: Option<&str>) -> Value {
    json!({"moveType": move_type, "fromUid": from_uid})
}

/// `addToPlaylist`: `uris` at the end, or before the row at `before`.
pub fn add_query(playlist_id: &str, uris: &[String], before: Option<&str>, top: bool) -> Value {
    let new_position = match (top, before) {
        (true, _) => position("TOP_OF_PLAYLIST", None),
        (false, Some(uid)) => position("BEFORE_UID", Some(uid)),
        (false, None) => position("BOTTOM_OF_PLAYLIST", None),
    };
    free::persisted(
        "addToPlaylist",
        MUTATION,
        json!({
            "playlistUri": playlist_uri(playlist_id),
            "playlistItemUris": uris,
            "newPosition": new_position,
            "interactionId": null,
        }),
    )
}

/// `removeFromPlaylist`: exactly the rows named.
pub fn remove_query(playlist_id: &str, uids: &[String]) -> Value {
    free::persisted(
        "removeFromPlaylist",
        MUTATION,
        json!({"playlistUri": playlist_uri(playlist_id), "uids": uids}),
    )
}

/// `moveItemsInPlaylist`: the row `uid` to the end, or before `before`.
pub fn move_query(playlist_id: &str, uid: &str, before: Option<&str>) -> Value {
    let new_position = match before {
        Some(before) => position("BEFORE_UID", Some(before)),
        None => position("BOTTOM_OF_PLAYLIST", None),
    };
    free::persisted(
        "moveItemsInPlaylist",
        MUTATION,
        json!({"playlistUri": playlist_uri(playlist_id), "uids": [uid], "newPosition": new_position}),
    )
}

async fn add(
    reader: &crate::web_player::Reader,
    id: &str,
    uris: &[String],
    at: Option<u32>,
) -> Result<Option<String>, ApiError> {
    if uris.is_empty() {
        return Ok(None);
    }
    let query = match at {
        None => add_query(id, uris, None, false),
        Some(0) => add_query(id, uris, None, true),
        Some(at) => {
            let list = rows(reader, id).await?;
            let before = list.rows.get(at as usize).map(|row| row.uid.as_str());
            add_query(id, uris, before, false)
        }
    };
    ask(reader, query).await?;
    revision(reader, id).await
}

/// Removes every copy of each song, as the Web API's removal by URI does.
async fn remove_songs(
    reader: &crate::web_player::Reader,
    id: &str,
    uris: &[String],
) -> Result<Option<String>, ApiError> {
    let list = rows(reader, id).await?;
    let uids: Vec<String> = list
        .rows
        .iter()
        .filter(|row| uris.contains(&row.uri))
        .map(|row| row.uid.clone())
        .collect();
    if uids.is_empty() {
        return Ok(list.revision);
    }
    ask(reader, remove_query(id, &uids)).await?;
    revision(reader, id).await
}

/// Moves the row at `from` to sit before the row now at `to`, as the Web
/// API's `range_start`/`insert_before` with one row.
async fn reorder(
    reader: &crate::web_player::Reader,
    id: &str,
    from: u32,
    to: u32,
) -> Result<Option<String>, ApiError> {
    let list = rows(reader, id).await?;
    let moved = list.rows.get(from as usize).ok_or_else(changed)?;
    if to == from || to == from + 1 {
        return Ok(list.revision);
    }
    let before = list.rows.get(to as usize).map(|row| row.uid.as_str());
    ask(reader, move_query(id, &moved.uid, before)).await?;
    revision(reader, id).await
}

/// Removes exactly the picked rows, leaving the song's other copies
/// (#937), by their uids in the list as it stands now. A row no longer
/// where it was is refused rather than another row removed on a guess.
pub async fn remove_rows(
    reader: &crate::web_player::Reader,
    id: &str,
    picked: &[RemovedRow],
) -> Result<Option<String>, ApiError> {
    let list = rows(reader, id).await?;
    let uris: Vec<String> = list.rows.iter().map(|row| row.uri.clone()).collect();
    let indices = crate::session_reads::removal_indices(&uris, picked).ok_or_else(changed)?;
    let uids: Vec<String> = indices
        .iter()
        .map(|(index, _)| list.rows[*index as usize].uid.clone())
        .collect();
    ask(reader, remove_query(id, &uids)).await?;
    revision(reader, id).await
}

fn source() -> Value {
    json!({"source": {"client": "WEBPLAYER"}})
}

fn list_changes(ops: Value) -> Value {
    json!({
        "deltas": [{"ops": ops, "info": source()}],
    })
}

/// A new playlist named `name`, as the web player creates one: the list
/// first, then its place at the top of the account's library. Bodies are
/// what the web player's protobuf `toJSON` writes, which leaves out empty
/// lists and false flags.
pub fn create_query(name: &str, description: &str) -> Value {
    let mut values = json!({"name": name});
    if !description.is_empty() {
        values["description"] = json!(description);
    }
    json!({"playlistChanges": {"create": true, "body": {
        "ops": [{"kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": {
            "newAttributes": {"values": values}}}],
    }}})
}

/// Puts `uri` at the top of the account's library, on the profile when
/// `public`.
pub fn library_add_query(username: &str, uri: &str, public: bool, timestamp_ms: i64) -> Value {
    let mut attributes = json!({
        "timestamp": timestamp_ms.to_string(),
    });
    if public {
        attributes["public"] = json!(true);
    }
    json!({"playlistChanges": {"rootlist": username, "body": list_changes(json!([
        {"kind": "ADD", "add": {"addFirst": true, "items": [{"uri": uri, "attributes": attributes}]}}
    ]))}})
}

/// Takes `uri` out of the account's library: for its own playlist, what
/// the web player's "Delete" does.
pub fn library_remove_query(username: &str, uri: &str) -> Value {
    json!({"playlistChanges": {"rootlist": username, "body": list_changes(json!([
        {"kind": "REM", "rem": {"itemsAsKey": true, "items": [{"uri": uri}]}}
    ]))}})
}

/// Shows `uri` on the profile or not, as the web player's "Add to
/// profile"/"Remove from profile": the Web API's `public`.
pub fn published_query(username: &str, uri: &str, public: bool) -> Value {
    json!({"playlistChanges": {"rootlist": username, "body": list_changes(json!([
        {"kind": "UPDATE_ITEM_ATTRIBUTES", "updateItemAttributes": {
            "item": {"uri": uri},
            "newAttributes": {"values": {"public": public}}}}
    ]))}})
}

/// A new name or description for the list. An empty description clears
/// it, as the web player does; `None` when nothing changes.
pub fn attributes_query(id: &str, name: Option<&str>, description: Option<&str>) -> Option<Value> {
    let mut values = json!({});
    let mut no_value = Vec::new();
    if let Some(name) = name {
        values["name"] = json!(name);
    }
    match description {
        Some("") => no_value.push("LIST_DESCRIPTION"),
        Some(description) => values["description"] = json!(description),
        None => {}
    }
    if name.is_none() && no_value.is_empty() && description.is_none() {
        return None;
    }
    let mut attributes = json!({"values": values});
    if !no_value.is_empty() {
        attributes["noValue"] = json!(no_value);
    }
    Some(
        json!({"playlistChanges": {"playlist": id, "body": list_changes(json!([
            {"kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": {"newAttributes": attributes}}
        ]))}}),
    )
}

async fn create(
    reader: &crate::web_player::Reader,
    account: Option<&str>,
    name: &str,
    public: bool,
    description: &str,
) -> Result<Playlist, ApiError> {
    let account = account.ok_or(ApiError::NotSignedIn)?;
    let created = ask(reader, create_query(name, description)).await?;
    let uri = created
        .get("uri")
        .and_then(Value::as_str)
        .filter(|uri| uri.starts_with("spotify:playlist:"))
        .ok_or_else(|| ApiError::Decode("Spotify sent no new playlist".into()))?
        .to_owned();
    let now = jiff::Timestamp::now().as_millisecond();
    ask(reader, library_add_query(account, &uri, public, now)).await?;
    Ok(Playlist {
        id: uri.rsplit(':').next().unwrap_or_default().to_owned(),
        name: name.to_owned(),
        description: (!description.is_empty()).then(|| description.to_owned()),
        owner: Owner {
            id: Some(account.to_owned()),
            uri: Some(format!("spotify:user:{account}")),
            ..Owner::default()
        },
        public: Some(public),
        uri,
        ..Playlist::default()
    })
}

async fn update(
    reader: &crate::web_player::Reader,
    account: Option<&str>,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    public: Option<bool>,
) -> Result<(), ApiError> {
    if name == Some("") {
        return Err(ApiError::Status {
            status: 400,
            message: "A playlist needs a name.".into(),
        });
    }
    if let Some(query) = attributes_query(id, name, description) {
        ask(reader, query).await?;
    }
    if let Some(public) = public {
        let account = account.ok_or(ApiError::NotSignedIn)?;
        ask(reader, published_query(account, &playlist_uri(id), public)).await?;
    }
    Ok(())
}

/// Deletes the account's own playlist `id` from its library. Only the
/// live test uses it, on the scratch playlist it made itself.
#[cfg(test)]
pub async fn delete_own(
    reader: &crate::web_player::Reader,
    account: &str,
    id: &str,
) -> Result<(), ApiError> {
    ask(reader, library_remove_query(account, &playlist_uri(id))).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::{PlayableItem, Track};

    const ROWS: &str = include_str!("testdata/free/playlist-rows.json");
    const ID: &str = "5cr4tchPl4yl1stF1xtur3";
    const A: &str = "spotify:track:3GWeTr5MQe85ybRi9nmCIc";
    const B: &str = "spotify:track:10EpXLXKHmNSVKvX7A5hg8";
    const C: &str = "spotify:track:0HKo7Vc6oePilkVcHcFr4v";
    const REVISION: &str = "AAAAB3NjcmF0Y2hyZXZpc2lvbg==";

    /// Runs `request` against a helper that answers `answers` in turn, and
    /// returns the response and every query the page was asked to send.
    fn run(answers: &[(u16, &str)], request: ApiRequest) -> (Option<ApiResponse>, Vec<Value>) {
        let dir = std::env::temp_dir().join(format!(
            "spotifast-free-edits-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("queries.jsonl");
        let answers: Vec<(u16, String)> = answers
            .iter()
            .map(|(status, body)| (*status, (*body).to_owned()))
            .collect();
        let player = crate::web_player::scripted(&answers, &log);
        let reader = player.reader();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let response = runtime.block_on(handle(&reader, Some("owner"), &request));
        player.shutdown();
        let sent = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        (response, sent)
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn read(offset: u32, limit: u32) -> Value {
        free::playlist_query(&playlist_uri(ID), offset, limit)
    }

    const OK: &str = r#"{"data":{"playlistMutation":{"__typename":"PlaylistMutationResponse"}}}"#;

    fn changed_with(response: Option<ApiResponse>) -> Result<Option<String>, ApiError> {
        match response {
            Some(ApiResponse::PlaylistItemsChanged { id, result, .. }) => {
                assert_eq!(id, ID);
                result
            }
            other => panic!("not a playlist change: {other:?}"),
        }
    }

    /// The rows and revision come from the playlist's own answer.
    #[test]
    fn a_page_names_each_row_by_its_uid() {
        let rows = page_rows(&serde_json::from_str(ROWS).unwrap()).unwrap();
        assert_eq!(rows.revision.as_deref(), Some(REVISION));
        assert_eq!(rows.total, 3);
        assert_eq!(
            rows.rows
                .iter()
                .map(|row| row.uri.as_str())
                .collect::<Vec<_>>(),
            [A, B, C]
        );
        assert_eq!(rows.rows[1].uid, "aa73ce749d66b5a67e2e");
    }

    /// Adding at the end sends `addToPlaylist` as the web player does, then
    /// reports the playlist's new revision as its snapshot.
    #[test]
    fn songs_are_added_at_the_end_with_the_new_snapshot() {
        let uris = vec![A.to_owned(), B.to_owned()];
        let (response, sent) = run(
            &[(200, OK), (200, ROWS)],
            ApiRequest::AddToPlaylist {
                playlist_id: ID.into(),
                playlist_name: "Scratch".into(),
                uris: uris.clone(),
                position: None,
            },
        );
        assert_eq!(
            sent,
            [
                json!({
                    "operationName": "addToPlaylist",
                    "variables": {
                        "playlistUri": playlist_uri(ID), "playlistItemUris": uris,
                        "newPosition": {"moveType": "BOTTOM_OF_PLAYLIST", "fromUid": null},
                        "interactionId": null,
                    },
                    "extensions": {"persistedQuery": {"version": 1, "sha256Hash": MUTATION}},
                }),
                read(0, 1),
            ]
        );
        let Some(ApiResponse::PlaylistItemsChanged { message, .. }) = &response else {
            panic!("{response:?}");
        };
        assert_eq!(message, "Added to Scratch");
        assert_eq!(changed_with(response).unwrap().as_deref(), Some(REVISION));
    }

    /// A position goes before the row there now, named by its uid; the top
    /// needs no row.
    #[test]
    fn songs_are_added_before_the_row_at_the_position() {
        let (response, sent) = run(
            &[(200, ROWS), (200, OK), (200, ROWS)],
            ApiRequest::AddToPlaylist {
                playlist_id: ID.into(),
                playlist_name: "Scratch".into(),
                uris: vec![C.to_owned()],
                position: Some(1),
            },
        );
        changed_with(response).unwrap();
        assert_eq!(sent[0], read(0, PAGE));
        assert_eq!(
            sent[1]["variables"]["newPosition"],
            json!({"moveType": "BEFORE_UID", "fromUid": "aa73ce749d66b5a67e2e"})
        );
        let (_, sent) = run(
            &[(200, OK), (200, ROWS)],
            ApiRequest::AddToPlaylist {
                playlist_id: ID.into(),
                playlist_name: "Scratch".into(),
                uris: vec![C.to_owned()],
                position: Some(0),
            },
        );
        assert_eq!(
            sent[0]["variables"]["newPosition"],
            json!({"moveType": "TOP_OF_PLAYLIST", "fromUid": null})
        );
    }

    /// Removing by song removes every row of it, as the Web API does.
    #[test]
    fn removing_a_song_removes_its_rows_by_uid() {
        let (response, sent) = run(
            &[(200, ROWS), (200, OK), (200, ROWS)],
            ApiRequest::RemoveFromPlaylist {
                playlist_id: ID.into(),
                uris: vec![C.to_owned()],
                snapshot_id: None,
            },
        );
        assert_eq!(changed_with(response).unwrap().as_deref(), Some(REVISION));
        assert_eq!(
            sent[1],
            json!({
                "operationName": "removeFromPlaylist",
                "variables": {"playlistUri": playlist_uri(ID), "uids": ["917b0d1eb385902afabd"]},
                "extensions": {"persistedQuery": {"version": 1, "sha256Hash": MUTATION}},
            })
        );
    }

    /// A move names the moved row and the row it goes before; moving past
    /// the last row goes to the bottom.
    #[test]
    fn a_move_names_the_row_and_where_it_goes() {
        let (response, sent) = run(
            &[(200, ROWS), (200, OK), (200, ROWS)],
            ApiRequest::ReorderPlaylist {
                playlist_id: ID.into(),
                range_start: 2,
                insert_before: 0,
                snapshot_id: None,
            },
        );
        changed_with(response).unwrap();
        assert_eq!(
            sent[1],
            json!({
                "operationName": "moveItemsInPlaylist",
                "variables": {
                    "playlistUri": playlist_uri(ID), "uids": ["917b0d1eb385902afabd"],
                    "newPosition": {"moveType": "BEFORE_UID", "fromUid": "672589e2ee75cc997d41"},
                },
                "extensions": {"persistedQuery": {"version": 1, "sha256Hash": MUTATION}},
            })
        );
        let (_, sent) = run(
            &[(200, ROWS), (200, OK), (200, ROWS)],
            ApiRequest::ReorderPlaylist {
                playlist_id: ID.into(),
                range_start: 0,
                insert_before: 3,
                snapshot_id: None,
            },
        );
        assert_eq!(
            sent[1]["variables"]["newPosition"],
            json!({"moveType": "BOTTOM_OF_PLAYLIST", "fromUid": null})
        );
    }

    /// A refused mutation is the app's usual failed change.
    #[test]
    fn a_refused_change_fails() {
        let (response, _) = run(
            &[(200, ROWS), (403, "nope")],
            ApiRequest::RemoveFromPlaylist {
                playlist_id: ID.into(),
                uris: vec![A.to_owned()],
                snapshot_id: None,
            },
        );
        assert!(matches!(
            changed_with(response),
            Err(ApiError::Status { status: 403, .. })
        ));
    }

    /// The duplicates check names the songs already in the list.
    #[test]
    fn duplicates_are_the_songs_already_listed() {
        let song = |uri: &str| {
            PlayableItem::Track(Track {
                uri: uri.into(),
                ..Track::default()
            })
        };
        let (response, sent) = run(
            &[(200, ROWS)],
            ApiRequest::CheckPlaylistDuplicates {
                playlist_id: ID.into(),
                playlist_name: "Scratch".into(),
                items: vec![song("spotify:track:new"), song(B)],
                position: Some(2),
            },
        );
        assert_eq!(sent, [read(0, PAGE)]);
        let Some(ApiResponse::PlaylistDuplicatesChecked {
            result, position, ..
        }) = response
        else {
            panic!("{response:?}");
        };
        assert_eq!(position, Some(2));
        assert_eq!(result.unwrap(), [B]);
    }

    /// A new playlist is made on the playlist service and put at the top
    /// of the library, then answered as the Web API's new playlist.
    #[test]
    fn a_new_playlist_is_made_and_listed() {
        let made = r#"{"uri":"spotify:playlist:5cr4tchPl4yl1stF1xtur3","revision":"AAAA"}"#;
        let (response, mut sent) = run(
            &[(200, made), (200, r#"{"revision":"AAAB"}"#)],
            ApiRequest::CreatePlaylist {
                name: "Scratch".into(),
                public: false,
                description: String::new(),
            },
        );
        assert_eq!(sent[0], create_query("Scratch", ""));
        assert_eq!(
            sent[0]["playlistChanges"]["body"]["ops"][0]["updateListAttributes"]["newAttributes"]["values"],
            json!({"name": "Scratch"})
        );
        let item =
            &mut sent[1]["playlistChanges"]["body"]["deltas"][0]["ops"][0]["add"]["items"][0];
        assert!(
            item["attributes"]["timestamp"]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .is_ok()
        );
        item["attributes"]["timestamp"] = json!("0");
        assert_eq!(
            sent[1],
            library_add_query("owner", &playlist_uri(ID), false, 0)
        );
        assert_eq!(sent[1]["playlistChanges"]["rootlist"], "owner");
        let Some(ApiResponse::PlaylistCreated(Ok(playlist))) = response else {
            panic!("{response:?}");
        };
        assert_eq!(playlist.id, ID);
        assert_eq!(playlist.uri, playlist_uri(ID));
        assert_eq!(playlist.name, "Scratch");
        assert_eq!(playlist.owner.id.as_deref(), Some("owner"));
        assert_eq!(playlist.public, Some(false));
    }

    /// A cover goes up as the web player sends one: the JPEG to the image
    /// service, then the picture it registers set as the list's own.
    #[test]
    fn a_cover_is_uploaded_and_set_as_the_list_picture() {
        let cover = crate::playlist_cover::Cover {
            jpeg: vec![0xff, 0xd8].into(),
            encoded: "/9g=".into(),
            uri: "file:///cover.jpg".into(),
        };
        let (response, sent) = run(
            &[
                (200, r#"{"picture":"cGlj"}"#),
                (200, r#"{"revision":"AAAB"}"#),
            ],
            ApiRequest::UploadPlaylistCover {
                id: ID.into(),
                request: 7,
                previous_urls: vec!["https://i.scdn.co/image/old".into()],
                cover: cover.clone(),
            },
        );
        assert_eq!(
            sent,
            [
                json!({"playlistImage": {"playlist": ID, "jpeg": "/9g="}}),
                json!({"playlistChanges": {"playlist": ID, "body": {
                    "deltas": [{
                        "ops": [{"kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": {
                            "newAttributes": {"values": {"picture": "cGlj"}}}}],
                        "info": {"source": {"client": "WEBPLAYER"}},
                    }],
                }}}),
            ]
        );
        let Some(ApiResponse::PlaylistCoverUploaded {
            id,
            request,
            previous_urls,
            result,
            ..
        }) = response
        else {
            panic!("{response:?}");
        };
        assert_eq!((id.as_str(), request), (ID, 7));
        assert_eq!(previous_urls, ["https://i.scdn.co/image/old"]);
        result.unwrap();
    }

    /// A refused upload is reported and the list is left alone.
    #[test]
    fn a_refused_cover_upload_changes_nothing() {
        let (response, sent) = run(
            &[(413, "too large")],
            ApiRequest::UploadPlaylistCover {
                id: ID.into(),
                request: 1,
                previous_urls: vec![],
                cover: crate::playlist_cover::Cover {
                    jpeg: vec![0xff].into(),
                    encoded: "/w==".into(),
                    uri: String::new(),
                },
            },
        );
        assert_eq!(sent.len(), 1);
        let Some(ApiResponse::PlaylistCoverUploaded { result, .. }) = response else {
            panic!("{response:?}");
        };
        assert!(result.is_err());
    }

    /// A rename and a new description go as one list change; `public`
    /// shows or hides the list on the profile.
    #[test]
    fn a_playlist_is_renamed_described_and_published() {
        let (response, sent) = run(
            &[
                (200, r#"{"revision":"AAAB"}"#),
                (200, r#"{"revision":"AAAC"}"#),
            ],
            ApiRequest::UpdatePlaylist {
                id: ID.into(),
                name: Some("Renamed".into()),
                description: Some(String::new()),
                public: Some(true),
            },
        );
        assert!(matches!(
            response,
            Some(ApiResponse::PlaylistUpdated { ref id, result: Ok(()) }) if id == ID
        ));
        assert_eq!(
            sent[0],
            json!({"playlistChanges": {"playlist": ID, "body": {
                "deltas": [{"ops": [{"kind": "UPDATE_LIST_ATTRIBUTES", "updateListAttributes": {
                    "newAttributes": {
                        "values": {"name": "Renamed"},
                        "noValue": ["LIST_DESCRIPTION"],
                    }}}], "info": {"source": {"client": "WEBPLAYER"}}}],
            }}})
        );
        assert_eq!(
            sent[1]["playlistChanges"]["body"]["deltas"][0]["ops"][0],
            json!({"kind": "UPDATE_ITEM_ATTRIBUTES", "updateItemAttributes": {
                "item": {"uri": playlist_uri(ID)},
                "newAttributes": {"values": {"public": true}}}})
        );
    }

    /// Only the public flag changes the library alone.
    #[test]
    fn only_publishing_sends_no_list_change() {
        let (response, sent) = run(
            &[(200, r#"{"revision":"AAAC"}"#)],
            ApiRequest::UpdatePlaylist {
                id: ID.into(),
                name: None,
                description: None,
                public: Some(false),
            },
        );
        assert!(matches!(
            response,
            Some(ApiResponse::PlaylistUpdated { result: Ok(()), .. })
        ));
        assert_eq!(sent, [published_query("owner", &playlist_uri(ID), false)]);
    }

    /// Live, on the real account: makes the scratch playlist, adds, moves
    /// and removes songs in it, renames it and deletes it, then checks the
    /// library lists exactly what it listed before. Touches nothing else.
    #[test]
    #[ignore = "needs the helper signed in to Spotify; writes a scratch playlist"]
    fn live_scratch_playlist_round_trip() {
        const SCRATCH: &str = "Spotifast Free test (safe to delete)";
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
        let account = runtime.block_on(async {
            let profile = ask(&reader, free::profile_query()).await?;
            let plan = ask(&reader, free::account_query()).await?;
            free::user(&profile, &plan)
        });
        let account = account.expect("the account").id;
        let library = |reader: &crate::web_player::Reader| -> Vec<(String, String)> {
            let mut all = Vec::new();
            let mut offset = 0;
            loop {
                let body = runtime
                    .block_on(ask(reader, free::library_query(offset, 50)))
                    .unwrap();
                let page = free::library_playlists(&body, offset).unwrap();
                all.extend(page.items.iter().map(|p| (p.id.clone(), p.name.clone())));
                match page.next {
                    Some(_) => offset += page.limit,
                    None => return all,
                }
            }
        };
        let before = library(&reader);
        assert!(
            !before
                .iter()
                .any(|(_, name)| name.starts_with("Spotifast Free test")),
            "a scratch playlist is already there: {before:?}"
        );
        eprintln!("library before: {} playlists", before.len());

        let created = runtime.block_on(handle(
            &reader,
            Some(&account),
            &ApiRequest::CreatePlaylist {
                name: SCRATCH.into(),
                public: false,
                description: String::new(),
            },
        ));
        let Some(ApiResponse::PlaylistCreated(Ok(scratch))) = created else {
            player.shutdown();
            panic!("create: {created:?}");
        };
        let id = scratch.id.clone();
        eprintln!("created {id}");
        let steps = || -> Result<(), String> {
            let order = || -> Result<Vec<String>, String> {
                let list = runtime
                    .block_on(rows(&reader, &id))
                    .map_err(|e| e.to_string())?;
                Ok(list.rows.into_iter().map(|row| row.uri).collect())
            };
            let change = |request: ApiRequest| -> Result<Option<String>, String> {
                match runtime.block_on(handle(&reader, Some(&account), &request)) {
                    Some(ApiResponse::PlaylistItemsChanged { result, .. }) => {
                        result.map_err(|e| e.to_string())
                    }
                    other => Err(format!("{other:?}")),
                }
            };
            let (a, b, c, d) = (A, B, C, "spotify:track:0DiWol3AO6WpXZgp0goxAV");
            let check = |want: &[&str]| -> Result<(), String> {
                let got = order()?;
                (got == want)
                    .then_some(())
                    .ok_or(format!("rows {got:?}, wanted {want:?}"))
            };
            let snapshot = change(ApiRequest::AddToPlaylist {
                playlist_id: id.clone(),
                playlist_name: SCRATCH.into(),
                uris: vec![a.into(), b.into(), c.into()],
                position: None,
            })?;
            eprintln!("added 3, snapshot {snapshot:?}");
            if snapshot.is_none() {
                return Err("no snapshot after adding".into());
            }
            check(&[a, b, c])?;
            change(ApiRequest::AddToPlaylist {
                playlist_id: id.clone(),
                playlist_name: SCRATCH.into(),
                uris: vec![d.into()],
                position: Some(1),
            })?;
            check(&[a, d, b, c])?;
            eprintln!("added at 1");
            match runtime.block_on(handle(
                &reader,
                Some(&account),
                &ApiRequest::CheckPlaylistDuplicates {
                    playlist_id: id.clone(),
                    playlist_name: SCRATCH.into(),
                    items: vec![PlayableItem::Track(Track {
                        uri: b.into(),
                        ..Track::default()
                    })],
                    position: None,
                },
            )) {
                Some(ApiResponse::PlaylistDuplicatesChecked {
                    result: Ok(found), ..
                }) if found == [b] => {}
                other => return Err(format!("duplicates: {other:?}")),
            }
            change(ApiRequest::ReorderPlaylist {
                playlist_id: id.clone(),
                range_start: 3,
                insert_before: 0,
                snapshot_id: None,
            })?;
            check(&[c, a, d, b])?;
            eprintln!("moved");
            change(ApiRequest::RemoveFromPlaylist {
                playlist_id: id.clone(),
                uris: vec![d.into()],
                snapshot_id: None,
            })?;
            check(&[c, a, b])?;
            runtime
                .block_on(remove_rows(
                    &reader,
                    &id,
                    &[RemovedRow {
                        uri: a.into(),
                        position: 1,
                        copy: 0,
                    }],
                ))
                .map_err(|e| e.to_string())?;
            check(&[c, b])?;
            eprintln!("removed");
            match runtime.block_on(handle(
                &reader,
                Some(&account),
                &ApiRequest::UpdatePlaylist {
                    id: id.clone(),
                    name: Some(format!("{SCRATCH} renamed")),
                    description: Some("Made by a Spotifast test.".into()),
                    public: None,
                },
            )) {
                Some(ApiResponse::PlaylistUpdated { result: Ok(()), .. }) => {}
                other => return Err(format!("update: {other:?}")),
            }
            let body = runtime
                .block_on(ask(&reader, free::playlist_query(&playlist_uri(&id), 0, 1)))
                .map_err(|e| e.to_string())?;
            let renamed = free::playlist(&body).map_err(|e| e.to_string())?;
            if renamed.name != format!("{SCRATCH} renamed")
                || renamed.description.as_deref() != Some("Made by a Spotifast test.")
            {
                return Err(format!(
                    "renamed to {:?} / {:?}",
                    renamed.name, renamed.description
                ));
            }
            eprintln!("renamed");
            Ok(())
        };
        let outcome = steps();
        // Always: empty and delete the scratch playlist, whatever failed.
        let emptied = runtime.block_on(async {
            let list = rows(&reader, &id).await?;
            let uids: Vec<String> = list.rows.into_iter().map(|row| row.uid).collect();
            if !uids.is_empty() {
                ask(&reader, remove_query(&id, &uids)).await?;
            }
            Ok::<_, ApiError>(())
        });
        let deleted = runtime.block_on(delete_own(&reader, &account, &id));
        let after = library(&reader);
        player.shutdown();
        eprintln!("library after: {} playlists", after.len());
        outcome.unwrap();
        emptied.unwrap();
        deleted.unwrap();
        assert_eq!(after, before, "the library is back as it was");
    }

    /// Picked rows are removed by their own uid, leaving the song's other
    /// copies, and a row no longer there removes nothing.
    #[test]
    fn picked_rows_are_removed_by_uid() {
        let dir = std::env::temp_dir().join(format!(
            "spotifast-free-rows-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("queries.jsonl");
        let player = crate::web_player::scripted(
            &[
                (200, ROWS.into()),
                (200, OK.into()),
                (200, ROWS.into()),
                (200, ROWS.into()),
            ],
            &log,
        );
        let reader = player.reader();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let picked = [RemovedRow {
            uri: B.into(),
            position: 1,
            copy: 0,
        }];
        let result = runtime.block_on(remove_rows(&reader, ID, &picked));
        assert_eq!(result.unwrap().as_deref(), Some(REVISION));
        let gone = [RemovedRow {
            uri: "spotify:track:gone".into(),
            position: 1,
            copy: 0,
        }];
        let refused = runtime.block_on(remove_rows(&reader, ID, &gone));
        assert!(matches!(refused, Err(ApiError::Status { status: 409, .. })));
        player.shutdown();
        let sent: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(sent.len(), 4, "the refused removal only read the list");
        assert_eq!(sent[1], remove_query(ID, &["aa73ce749d66b5a67e2e".into()]));
    }
}
