//! Library edits for a Free sign-in, through Spotify's web player: saving
//! albums, shows and songs (`SetSaved`) and following playlists
//! (`FollowPlaylist`), which the Web API refuses for a Free account's token.

use serde_json::Value;

use crate::backend::{ApiRequest, ApiResponse};

/// Answers `request` for a Free sign-in, or `None` when this module does
/// not cover it.
pub async fn handle(
    _reader: &crate::web_player::Reader,
    _account: Option<&str>,
    _request: &ApiRequest,
) -> Option<ApiResponse> {
    None
}

/// Saves these albums, shows, episodes, artists or songs to the library,
/// or removes them.
pub fn library_change_query(_uris: &[String], _saved: bool) -> Value {
    Value::Null
}

/// Adds a playlist to the top of the account's playlists, or removes it.
pub fn rootlist_change_query(_username: &str, _uri: &str, _follow: bool, _now_ms: u64) -> Value {
    Value::Null
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
