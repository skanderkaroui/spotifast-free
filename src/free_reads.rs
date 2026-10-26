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
use crate::api::models::{Image, User};

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
        player.shutdown();
        assert!(!user.id.is_empty());
        assert!(user.product.is_some());
        eprintln!(
            "signed in through the web player: plan {:?}, name present: {}",
            user.product,
            user.display_name.is_some()
        );
    }
}
