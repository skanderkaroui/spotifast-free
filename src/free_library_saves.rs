//! Library edits for a Free sign-in, through Spotify's web player: saving
//! albums, shows and songs (`SetSaved`) and following playlists
//! (`FollowPlaylist`), which the Web API refuses for a Free account's token.

use serde_json::{Value, json};

use crate::api::ApiError;
use crate::backend::{ApiRequest, ApiResponse};
use crate::free_reads::{answer, persisted};

/// Answers `request` for a Free sign-in, or `None` when this module does
/// not cover it.
pub async fn handle(
    reader: &crate::web_player::Reader,
    account: Option<&str>,
    request: &ApiRequest,
) -> Option<ApiResponse> {
    let send = |query: Value| {
        let pending = reader.query(&query);
        async move { answer(pending.await).map(|_| ()) }
    };
    Some(match request {
        ApiRequest::SetSaved { uris, saved } => ApiResponse::SavedChanged {
            uris: uris.clone(),
            saved: *saved,
            result: send(library_change_query(uris, *saved)).await,
        },
        ApiRequest::FollowPlaylist { id, follow } => ApiResponse::PlaylistFollowChanged {
            id: id.clone(),
            followed: *follow,
            result: match account {
                Some(account) => {
                    send(rootlist_change_query(
                        account,
                        &format!("spotify:playlist:{id}"),
                        *follow,
                        now_ms(),
                    ))
                    .await
                }
                None => Err(ApiError::NotSignedIn),
            },
        },
        _ => return None,
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Saves these albums, shows, episodes, artists or songs to the library,
/// or removes them.
pub fn library_change_query(uris: &[String], saved: bool) -> Value {
    persisted(
        if saved {
            "addToLibrary"
        } else {
            "removeFromLibrary"
        },
        "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
        json!({"libraryItemUris": uris}),
    )
}

/// Adds a playlist to the top of the account's playlists, or removes it.
pub fn rootlist_change_query(username: &str, uri: &str, follow: bool, now_ms: u64) -> Value {
    let op = if follow {
        json!({"kind": "ADD", "add": {
            "addFirst": true,
            "items": [{"uri": uri, "attributes": {
                "timestamp": now_ms.to_string(),
                "formatAttributes": [],
                "availableSignals": [],
            }}],
        }})
    } else {
        json!({"kind": "REM", "rem": {"itemsAsKey": true, "items": [{"uri": uri}]}})
    };
    json!({"playlistChanges": {"rootlist": username, "body": {
        "deltas": [{"ops": [op], "info": {"source": {"client": "WEBPLAYER"}}}],
        "wantResultingRevisions": false,
        "wantSyncResult": false,
        "nonces": [],
    }}})
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::free_reads as free;

    /// An album and a Spotify editorial playlist the test saves, then
    /// removes again. Never a song, an artist or a user.
    const ALBUM: &str = "spotify:album:4m2880jivSbbyEGAKfITCa";
    const PLAYLIST_ID: &str = "37i9dQZF1DX4sWSpwq3LiO";

    /// Undoes whatever the test changed, even when an assertion fails.
    struct Undo<'a> {
        runtime: &'a tokio::runtime::Runtime,
        reader: &'a crate::web_player::Reader,
        account: &'a str,
        album_saved: bool,
        playlist_followed: bool,
    }

    impl Drop for Undo<'_> {
        fn drop(&mut self) {
            if self.album_saved {
                let query = library_change_query(&[ALBUM.to_owned()], false);
                let done = self.runtime.block_on(self.reader.query(&query));
                eprintln!("cleanup: album removed: {:?}", answer(done).is_ok());
            }
            if self.playlist_followed {
                let query = rootlist_change_query(
                    self.account,
                    &format!("spotify:playlist:{PLAYLIST_ID}"),
                    false,
                    now_ms(),
                );
                let done = self.runtime.block_on(self.reader.query(&query));
                eprintln!("cleanup: playlist unfollowed: {:?}", answer(done).is_ok());
            }
        }
    }

    #[test]
    #[ignore = "needs the helper signed in to Spotify; saves then removes one album and follows then unfollows one playlist"]
    fn saving_and_following_round_trip_through_the_helper() {
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
        let reader = player.reader();
        let read = |query: Value| answer(runtime.block_on(reader.query(&query))).unwrap();
        let account = free::user(&read(free::profile_query()), &read(free::account_query()))
            .unwrap()
            .id;
        let playlist = format!("spotify:playlist:{PLAYLIST_ID}");
        // `areEntitiesInLibrary` answers for albums but not for followed
        // playlists, so the playlist is looked up in the whole library.
        let in_library = |read: &dyn Fn(Value) -> Value| {
            let album =
                free::contains(&read(free::contains_query(&[ALBUM.to_owned()])), 1).unwrap()[0];
            let mut offset = 0;
            let mut followed = false;
            loop {
                let page = free::library_playlists(&read(free::library_query(offset, 50)), offset)
                    .unwrap();
                followed |= page.items.iter().any(|item| item.uri == playlist);
                if page.next.is_none() {
                    break;
                }
                offset += page.limit;
            }
            vec![album, followed]
        };
        let before = in_library(&read);
        eprintln!(
            "before: album saved {}, playlist followed {}",
            before[0], before[1]
        );
        assert_eq!(
            before,
            vec![false, false],
            "both must start out of the library"
        );

        let mut undo = Undo {
            runtime: &runtime,
            reader: &reader,
            account: &account,
            album_saved: false,
            playlist_followed: false,
        };
        let ask = |request: ApiRequest| {
            runtime
                .block_on(handle(&reader, Some(&account), &request))
                .unwrap()
        };

        undo.album_saved = true;
        let ApiResponse::SavedChanged {
            result,
            saved: true,
            ..
        } = ask(ApiRequest::SetSaved {
            uris: vec![ALBUM.to_owned()],
            saved: true,
        })
        else {
            panic!()
        };
        result.unwrap();
        undo.playlist_followed = true;
        let ApiResponse::PlaylistFollowChanged {
            result,
            followed: true,
            ..
        } = ask(ApiRequest::FollowPlaylist {
            id: PLAYLIST_ID.into(),
            follow: true,
        })
        else {
            panic!()
        };
        result.unwrap();
        let during = in_library(&read);
        eprintln!(
            "after saving: album saved {}, playlist followed {}",
            during[0], during[1]
        );

        let ApiResponse::SavedChanged {
            result,
            saved: false,
            ..
        } = ask(ApiRequest::SetSaved {
            uris: vec![ALBUM.to_owned()],
            saved: false,
        })
        else {
            panic!()
        };
        result.unwrap();
        undo.album_saved = false;
        let ApiResponse::PlaylistFollowChanged {
            result,
            followed: false,
            ..
        } = ask(ApiRequest::FollowPlaylist {
            id: PLAYLIST_ID.into(),
            follow: false,
        })
        else {
            panic!()
        };
        result.unwrap();
        undo.playlist_followed = false;
        drop(undo);

        let after = in_library(&read);
        eprintln!(
            "after cleanup: album saved {}, playlist followed {}",
            after[0], after[1]
        );
        assert_eq!(
            during,
            vec![true, true],
            "both were in the library while saved"
        );
        assert_eq!(after, before, "the account is back as it was");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Saving and removing send the web player's own library mutations
    /// (REA: web-player bundle, `addToLibrary`/`removeFromLibrary`).
    #[test]
    fn saving_sends_the_web_players_library_mutation() {
        let uris = vec![
            "spotify:album:4m2880jivSbbyEGAKfITCa".to_owned(),
            "spotify:show:5CfCWKI5pZ28U0uOzXkDHe".to_owned(),
        ];
        let extensions = json!({"persistedQuery": {
            "version": 1,
            "sha256Hash": "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
        }});
        assert_eq!(
            library_change_query(&uris, true),
            json!({
                "operationName": "addToLibrary",
                "variables": {"libraryItemUris": uris},
                "extensions": extensions,
            })
        );
        assert_eq!(
            library_change_query(&uris, false),
            json!({
                "operationName": "removeFromLibrary",
                "variables": {"libraryItemUris": uris},
                "extensions": extensions,
            })
        );
    }

    /// Following a playlist adds it to the top of the account's rootlist,
    /// and unfollowing removes it by URI, as the web player's rootlist
    /// delta does (it refuses playlist URIs in `addToLibrary`).
    #[test]
    fn following_a_playlist_changes_the_rootlist() {
        let uri = "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M";
        assert_eq!(
            rootlist_change_query("listener1", uri, true, 1_700_000_000_000),
            json!({"playlistChanges": {"rootlist": "listener1", "body": {
                "deltas": [{
                    "ops": [{"kind": "ADD", "add": {
                        "addFirst": true,
                        "items": [{"uri": uri, "attributes": {
                            "timestamp": "1700000000000",
                            "formatAttributes": [],
                            "availableSignals": [],
                        }}],
                    }}],
                    "info": {"source": {"client": "WEBPLAYER"}},
                }],
                "wantResultingRevisions": false,
                "wantSyncResult": false,
                "nonces": [],
            }}})
        );
        assert_eq!(
            rootlist_change_query("listener1", uri, false, 1_700_000_000_000),
            json!({"playlistChanges": {"rootlist": "listener1", "body": {
                "deltas": [{
                    "ops": [{"kind": "REM", "rem": {
                        "itemsAsKey": true,
                        "items": [{"uri": uri}],
                    }}],
                    "info": {"source": {"client": "WEBPLAYER"}},
                }],
                "wantResultingRevisions": false,
                "wantSyncResult": false,
                "nonces": [],
            }}})
        );
    }
}
