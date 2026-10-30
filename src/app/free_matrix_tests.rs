//! Acceptance matrix: every Free/Premium difference in `capabilities`,
//! checked through the app's real paths for both plans.
#![cfg(target_os = "macos")]

use super::*;
use crate::web_player::{AD_URI, DEVICE_ID};

const SONG: &str = "spotify:track:a";

fn app_for(name: &str, free: bool, uri: &str) -> App {
    let root = std::env::temp_dir().join(format!(
        "spotifast-matrix-{name}-{free}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::new(
        &Waker::default(),
        AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        },
        Settings::default(),
        AppOptions {
            media_controls: false,
            restore_sign_in: false,
            tray: false,
            autostart: false,
        },
    );
    app.reveal_theme_changes = false;
    app.handle_playback(LocalPlayback::Ready {
        device_id: if free { DEVICE_ID } else { "local" }.into(),
    });
    app.handle_local(playing(uri, 1));
    app.backend.take_player_commands();
    app
}

fn playing(uri: &str, sequence: u64) -> LocalState {
    LocalState {
        connected: true,
        playback: Playback::Playing,
        track: Some(crate::player::LocalTrack {
            uri: uri.into(),
            title: "song".into(),
            duration_ms: 200_000,
            ..Default::default()
        }),
        track_sequence: sequence,
        ..LocalState::default()
    }
}

fn run(app: &mut App, action: Action) -> Vec<PlayerCommand> {
    app.actions.push(action);
    app.apply_actions(&egui::Context::default());
    app.backend.take_player_commands()
}

fn both(name: &str, check: impl Fn(bool, &mut App)) {
    for free in [true, false] {
        let mut app = app_for(name, free, SONG);
        check(free, &mut app);
        app.backend.shutdown();
    }
}

#[test]
fn play_context_from_a_song() {
    both("play-view", |free, app| {
        let sent = run(
            app,
            Action::PlayFromRow {
                context: RowContext::View {
                    uris: vec!["spotify:track:b".to_string(), SONG.to_string()].into(),
                    context_uri: "spotify:playlist:p".into(),
                    editable_playlist: None,
                },
                uri: SONG.into(),
                index: 1,
            },
        );
        let Some(PlayerCommand::Load(spec)) = sent.last() else {
            panic!("free={free}: expected a load, got {sent:?}");
        };
        if free {
            assert_eq!(spec.context_uri.as_deref(), Some("spotify:playlist:p"));
            assert_eq!(spec.offset_uri.as_deref(), Some(SONG));
        } else {
            assert!(spec.context_uri.is_none(), "{spec:?}");
            assert!(spec.uris.contains(&SONG.to_string()), "{spec:?}");
        }
    });
    both("play-context", |free, app| {
        let sent = run(
            app,
            Action::PlayFromRow {
                context: RowContext::Context {
                    uri: "spotify:album:x".into(),
                    editable_playlist: None,
                },
                uri: SONG.into(),
                index: 0,
            },
        );
        let Some(PlayerCommand::Load(spec)) = sent.last() else {
            panic!("free={free}: expected a load, got {sent:?}");
        };
        assert_eq!(spec.context_uri.as_deref(), Some("spotify:album:x"));
        assert_eq!(spec.offset_uri.as_deref(), Some(SONG));
    });
}

#[test]
fn transport_controls_reach_both_engines() {
    both("transport", |free, app| {
        let next = run(app, Action::Next);
        assert!(
            next.iter().any(|c| matches!(c, PlayerCommand::Next)),
            "free={free}"
        );
        let prev = run(app, Action::Previous);
        assert!(
            prev.iter().any(|c| matches!(c, PlayerCommand::Previous)),
            "free={free}"
        );
        let seek = run(app, Action::Seek(5_000));
        assert!(
            seek.iter().any(|c| matches!(c, PlayerCommand::Seek(5_000))),
            "free={free}"
        );
        let shuffle = run(app, Action::SetShuffle(true));
        assert!(
            shuffle
                .iter()
                .any(|c| matches!(c, PlayerCommand::Shuffle(true))),
            "free={free}"
        );
        let repeat = run(app, Action::SetRepeat(RepeatMode::Track));
        assert!(
            repeat
                .iter()
                .any(|c| matches!(c, PlayerCommand::Repeat(RepeatMode::Track))),
            "free={free}"
        );
    });
}

#[test]
fn transport_is_blocked_during_a_free_ad() {
    let mut app = app_for("ad-transport", true, AD_URI);
    for action in [Action::Next, Action::Previous, Action::Seek(5_000)] {
        assert!(run(&mut app, action).is_empty());
    }
    app.backend.shutdown();
}

#[test]
fn free_skip_next_restriction_blocks_next() {
    let mut app = app_for("restrict-next", true, SONG);
    let mut state = playing(SONG, 1);
    state.restrictions.skip_next = true;
    app.handle_local(state);
    app.backend.take_player_commands();
    let sent = run(&mut app, Action::Next);
    assert!(
        !sent.iter().any(|c| matches!(c, PlayerCommand::Next)),
        "{sent:?}"
    );
    app.backend.shutdown();
}

/// Live run on 0.13.2: Previous straight after Next on a list's first song
/// was dropped, because Spotify's state for the skipped-to song (allowing
/// it) had not arrived and the first song's "no previous track" still held.
#[test]
fn free_previous_right_after_next_is_not_held_by_the_old_song() {
    let mut app = app_for("prev-after-next", true, SONG);
    let mut first = playing(SONG, 1);
    first.restrictions.skip_prev = true;
    app.handle_local(first);
    app.backend.take_player_commands();
    assert!(
        run(&mut app, Action::Previous).is_empty(),
        "the first song itself has no previous"
    );
    let sent = run(&mut app, Action::Next);
    assert!(
        sent.iter().any(|c| matches!(c, PlayerCommand::Next)),
        "{sent:?}"
    );
    let sent = run(&mut app, Action::Previous);
    assert!(
        sent.iter().any(|c| matches!(c, PlayerCommand::Previous)),
        "{sent:?}"
    );
    app.backend.shutdown();
}

/// Live run on 0.14.0: Spotify's report for the song just skipped from,
/// still saying "no previous track", came in after Next. It must not hold
/// Previous again before the skipped-to song's own report arrives.
#[test]
fn free_previous_is_not_held_by_a_late_report_for_the_skipped_from_song() {
    let mut app = app_for("prev-late-report", true, SONG);
    let mut first = playing(SONG, 1);
    first.restrictions.skip_prev = true;
    app.handle_local(first.clone());
    app.backend.take_player_commands();
    run(&mut app, Action::Next);
    // The skipped-from song's report, late.
    app.handle_local(first);
    app.backend.take_player_commands();
    let sent = run(&mut app, Action::Previous);
    assert!(
        sent.iter().any(|c| matches!(c, PlayerCommand::Previous)),
        "{sent:?}"
    );
    app.backend.shutdown();
}

#[test]
fn free_next_right_after_previous_is_not_held_by_the_old_song() {
    let mut app = app_for("next-after-prev", true, SONG);
    let mut last = playing(SONG, 1);
    last.restrictions.skip_next = true;
    app.handle_local(last);
    app.backend.take_player_commands();
    run(&mut app, Action::Previous);
    let sent = run(&mut app, Action::Next);
    assert!(
        sent.iter().any(|c| matches!(c, PlayerCommand::Next)),
        "{sent:?}"
    );
    app.backend.shutdown();
}

/// Live run on 0.14.1: Next pressed right after a playlist started, before
/// Spotify's first report for it. The app still knew the song before the
/// playlist, so the playlist's first song ("no previous track") reported
/// late looked like news and held Previous.
#[test]
fn free_previous_is_not_held_when_next_beats_the_new_lists_first_report() {
    let mut app = app_for("prev-before-first-report", true, SONG);
    app.handle_local(playing(SONG, 1));
    app.backend.take_player_commands();
    run(&mut app, Action::Next);
    // The newly loaded list's first song, reported after Next was sent.
    let mut first_of_list = playing("spotify:track:00000000000000000000aa", 1);
    first_of_list.restrictions.skip_prev = true;
    app.handle_local(first_of_list);
    app.backend.take_player_commands();
    let sent = run(&mut app, Action::Previous);
    assert!(
        sent.iter().any(|c| matches!(c, PlayerCommand::Previous)),
        "{sent:?}"
    );
    app.backend.shutdown();
}

#[test]
fn volume_is_sent_and_saved_only_for_premium() {
    for action in [
        Action::SetVolume(40),
        Action::VolumeBy(-10),
        Action::ToggleMute,
    ] {
        both("volume", |free, app| {
            app.settings.volume = 20_000;
            let sent = run(app, action.clone());
            let Some(PlayerCommand::Volume(level)) =
                sent.iter().find(|c| matches!(c, PlayerCommand::Volume(_)))
            else {
                panic!("free={free} {action:?}: no volume sent, got {sent:?}");
            };
            let expected = if free { 20_000 } else { *level };
            assert_eq!(app.settings.volume, expected, "free={free} {action:?}");
        });
    }
}

#[test]
fn add_to_queue() {
    both("queue-add", |free, app| {
        app.local.playback = Playback::Stopped;
        let sent = run(
            app,
            Action::AddToQueue {
                uri: "spotify:track:b".into(),
                label: "b".into(),
            },
        );
        if free {
            assert!(
                matches!(sent.as_slice(), [PlayerCommand::AddToQueue(u)] if u == "spotify:track:b"),
                "{sent:?}"
            );
        } else {
            // Premium with no active local playback writes through the Web API.
            assert!(sent.is_empty(), "{sent:?}");
            assert!(!app.backend.take_queue_requests().is_empty());
        }
    });
    both("queue-many", |free, app| {
        let sent = run(
            app,
            Action::QueueMany {
                songs: vec![
                    ("spotify:track:b".into(), "b".into()),
                    ("spotify:track:c".into(), "c".into()),
                ],
            },
        );
        let added: Vec<_> = sent
            .iter()
            .filter_map(|c| match c {
                PlayerCommand::AddToQueue(u) => Some(u.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(added, ["spotify:track:b", "spotify:track:c"], "free={free}");
    });
}

#[test]
fn queue_editing_is_premium_only() {
    both("queue-edit", |free, app| {
        app.manual_queue = vec!["spotify:track:b".into()];
        app.queue = Loadable::Loaded(Queue {
            currently_playing: None,
            queue: vec![PlayableItem::Track(Track {
                uri: "spotify:track:b".into(),
                ..Default::default()
            })],
        });
        assert_eq!(app.can_clear_queue(), !free);
        assert_eq!(app.queue_locally_reorderable(), !free);
        assert_eq!(app.can_remove_from_queue(0), !free);
        let sent = run(app, Action::MoveInQueue { from: 0, to: 0 });
        if free {
            assert!(sent.is_empty(), "{sent:?}");
        }
    });
}

/// Live, 2026-10-26: a Free sign-in read its Made For shelf through the web
/// player, yet Home kept its placeholders. The helper became ready after
/// Home asked, which asked again, and the first answer was dropped as
/// stale while the second never came.
#[test]
fn a_free_home_shows_its_made_for_you_shelf() {
    let root = std::env::temp_dir().join(format!("spotifast-free-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::new(
        &Waker::default(),
        AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        },
        Settings::default(),
        AppOptions {
            media_controls: false,
            restore_sign_in: false,
            tray: false,
            autostart: false,
        },
    );
    app.load_home(false);
    let asked = app.home.made_for_you_generation;
    app.handle_playback(LocalPlayback::Ready {
        device_id: DEVICE_ID.into(),
    });
    let body: serde_json::Value =
        serde_json::from_str(include_str!("../testdata/free/home.json")).unwrap();
    let (made_for_you, top_mixes) = crate::free_reads::home_shelves(&body).unwrap();
    for generation in [asked, app.home.made_for_you_generation] {
        app.handle_api(ApiResponse::MadeForYou {
            generation,
            personalized: true,
            result: Ok(made_for_you.clone()),
            top_mixes: Ok(top_mixes.clone()),
        });
    }
    let shown: Vec<_> = app
        .home
        .made_for_you
        .get()
        .expect("the shelf has its cards")
        .iter()
        .map(|playlist| playlist.name.clone())
        .collect();
    assert_eq!(shown, ["Mix Uk2", "Mix iP5", "Mix Opz"]);
    assert_eq!(app.home.top_mixes.get().map(Vec::len), Some(3));
    app.backend.shutdown();
}

/// Hiding songs needs the Premium playback session. A Free account's player
/// is the web player, so it is never asked; live it failed with "Hiding
/// songs needs playback on this computer" on every launch.
#[test]
fn free_never_reads_hidden_songs() {
    both("hidden-songs", |free, app| {
        app.load_hidden_songs();
        assert_eq!(app.hidden_songs_requested, !free, "free={free}");
    });
}

/// Live, 2026-10-26: a song in Next up whose lookup failed was asked for
/// again on each of the helper's state reports, 125,000 times in 30 s. A
/// failed lookup waits until the song is asked for in a new way.
#[test]
fn a_failed_next_up_lookup_is_not_repeated_on_every_report() {
    let mut app = app_for("upcoming-failed", true, SONG);
    let mut state = playing(SONG, 1);
    state.upcoming = vec!["spotify:track:b".into()];
    app.handle_local(state.clone());
    assert!(app.track_requests.contains("b"), "asked once");
    app.handle_api(ApiResponse::Track {
        id: "b".into(),
        result: Err(crate::api::ApiError::Network("no answer".into())),
    });
    for sequence in 0..50 {
        // Each report differs (position, next up rotating back), as live.
        let mut report = state.clone();
        report.position_ms = sequence * 1000;
        report.upcoming = if sequence % 2 == 0 {
            vec!["spotify:track:b".into(), "spotify:track:c".into()]
        } else {
            vec!["spotify:track:b".into()]
        };
        app.handle_local(report);
        app.handle_api(ApiResponse::Track {
            id: "c".into(),
            result: Err(crate::api::ApiError::Network("no answer".into())),
        });
    }
    assert!(
        !app.track_requests.contains("b"),
        "a failed song is not asked for again on each report"
    );
    app.backend.shutdown();
}

#[test]
fn free_next_up_comes_from_upcoming() {
    let mut app = app_for("upcoming", true, SONG);
    let mut state = playing(SONG, 1);
    state.upcoming = vec!["spotify:track:b".into(), "spotify:track:c".into()];
    app.handle_local(state);
    let uris: Vec<_> = app
        .queue
        .get()
        .expect("loaded")
        .queue
        .iter()
        .map(|item| item.uri().to_string())
        .collect();
    assert_eq!(uris, ["spotify:track:b", "spotify:track:c"]);
    app.backend.shutdown();
}

#[test]
fn radio() {
    let seed = "spotify:track:seed";
    for open_page in [true, false] {
        both("radio", |free, app| {
            let action = if open_page {
                Action::Open(Page::Radio(seed.into()))
            } else {
                Action::OpenSongRadio {
                    uri: seed.into(),
                    track: Box::new(Track {
                        uri: seed.into(),
                        ..Default::default()
                    }),
                }
            };
            let sent = run(app, action);
            if free {
                assert!(
                    matches!(sent.as_slice(), [PlayerCommand::Radio(s)] if s == seed),
                    "{sent:?}"
                );
                assert_ne!(app.page(), &Page::Radio(seed.into()));
            } else {
                assert!(
                    !sent.iter().any(|c| matches!(c, PlayerCommand::Radio(_))),
                    "{sent:?}"
                );
                assert_eq!(app.page(), &Page::Radio(seed.into()));
            }
        });
    }
}

#[test]
fn device_transfer_is_premium_only() {
    both("transfer", |free, app| {
        app.backend.take_transfer_requests();
        run(app, Action::Transfer("phone".into()));
        let sent = app.backend.take_transfer_requests();
        if free {
            assert!(sent.is_empty(), "{sent:?}");
        } else {
            assert!(
                matches!(sent.as_slice(), [ApiRequest::Transfer { device_id, .. }] if device_id == "phone"),
                "{sent:?}"
            );
        }
    });
}

#[test]
fn capabilities_by_plan() {
    both("caps", |free, app| {
        let caps = app.capabilities();
        for (name, value) in [
            ("devices", caps.devices),
            ("sound_effects", caps.sound_effects),
            ("visualizers", caps.visualizers),
            ("engine_settings", caps.engine_settings),
            ("prerelease", caps.prerelease),
        ] {
            assert_eq!(value, !free, "free={free} {name}");
        }
        for (name, value) in [
            ("skip_next", caps.skip_next),
            ("skip_prev", caps.skip_prev),
            ("seek", caps.seek),
            ("shuffle", caps.shuffle),
            ("repeat", caps.repeat),
            ("volume", caps.volume),
            ("queue_add", caps.queue_add),
            ("radio", caps.radio),
        ] {
            assert!(value, "free={free} {name}");
        }
        assert_eq!(caps.queue_edit, !free);
    });
}

#[test]
fn ads_are_never_resumed_saved_or_recorded() {
    let mut app = app_for("ads", true, SONG);
    app.handle_local(playing(AD_URI, 2));
    app.save_session();
    assert_eq!(app.resume_track.as_deref(), Some(SONG));

    app.backend.take_player_commands();
    run(&mut app, Action::ToggleSaved(AD_URI.into()));
    let queue = std::sync::Arc::new(std::sync::Mutex::new(vec![ControlCommand::ToggleSaved]));
    app.control_commands = Some(queue);
    app.handle_control_commands();
    app.apply_actions(&egui::Context::default());
    assert!(!app.saved.contains_key(AD_URI));
    assert!(!app.saved_writes.contains_key(AD_URI));

    app.note_listening();
    assert!(app.listening.is_none());
    assert!(
        app.plays
            .plays()
            .iter()
            .all(|play| play.track.uri != AD_URI)
    );
    app.backend.shutdown();
}

#[test]
fn free_flip_to_premium_restores_premium_caps() {
    let mut app = app_for("flip", true, SONG);
    assert!(!app.capabilities().devices);
    app.handle_playback(LocalPlayback::Unavailable);
    app.handle_playback(LocalPlayback::Ready {
        device_id: "local".into(),
    });
    let caps = app.capabilities();
    assert!(caps.devices && caps.sound_effects && caps.engine_settings && caps.queue_edit);
    run(&mut app, Action::Transfer("phone".into()));
    assert!(!app.backend.take_transfer_requests().is_empty());
    app.backend.shutdown();
}

/// The words the settings page paints after filtering for `query`.
fn settings_text(app: &mut App, query: &str) -> String {
    let ctx = egui::Context::default();
    app.attach(&ctx);
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new("settings-filter"), query.to_owned());
    });
    let mut text = String::new();
    for _ in 0..2 {
        text.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 2200.0),
                )),
                ..Default::default()
            },
            |ui| crate::ui::settings::show(app, ui),
        );
        output.textures_delta.clear();
        fn walk(shape: &egui::epaint::Shape, text: &mut String) {
            match shape {
                egui::epaint::Shape::Text(shape) => {
                    text.push_str(&shape.galley.job.text);
                    text.push('\n');
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, text)),
                _ => {}
            }
        }
        for shape in &output.shapes {
            walk(&shape.shape, &mut text);
        }
    }
    text
}

/// Local files play through librespot, which Spotify Free's web player
/// never uses, so the folder setting is Premium's alone.
#[test]
fn local_files_setting_is_premium_only() {
    both("local-files", |free, app| {
        let text = settings_text(app, "Local files");
        assert_eq!(text.contains("Choose folder"), !free, "free={free}\n{text}");
    });
}

/// Clicking a row of Next up skips to it. On Free that is Spotify's Next,
/// once per row, so it must follow the same holds as the Next button: no
/// skips through an ad, none when Spotify holds Next, and the rows stay.
#[test]
fn free_queue_click_follows_the_next_buttons_holds() {
    let rows = ["spotify:track:b", "spotify:track:c"];
    let click = || Action::PlayFromRow {
        context: RowContext::Queue,
        uri: "spotify:track:c".into(),
        index: 1,
    };
    let queue = || {
        Loadable::Loaded(Queue {
            currently_playing: None,
            queue: rows
                .iter()
                .map(|uri| {
                    crate::api::models::PlayableItem::Track(crate::api::models::Track {
                        uri: (*uri).into(),
                        ..Default::default()
                    })
                })
                .collect(),
        })
    };
    // During an ad.
    let mut app = app_for("queue-click-ad", true, AD_URI);
    app.queue = queue();
    assert!(run(&mut app, click()).is_empty(), "no skips through an ad");
    assert_eq!(app.queue.get().map(|q| q.queue.len()), Some(2), "rows stay");
    app.backend.shutdown();
    // Spotify holding Next.
    let mut app = app_for("queue-click-held", true, SONG);
    let mut state = playing(SONG, 2);
    state.restrictions.skip_next = true;
    app.handle_local(state);
    app.backend.take_player_commands();
    app.queue = queue();
    assert!(run(&mut app, click()).is_empty(), "Next is held");
    app.backend.shutdown();
    // Allowed: one Next per row up to the chosen one.
    let mut app = app_for("queue-click-ok", true, SONG);
    app.queue = queue();
    let sent = run(&mut app, click());
    assert_eq!(
        sent.iter()
            .filter(|c| matches!(c, PlayerCommand::Next))
            .count(),
        2,
        "{sent:?}"
    );
    app.backend.shutdown();
}

/// A song queued on Free leaves the queue once Spotify plays it, so the
/// next launch does not queue it again. Spotify's state names it as the
/// playing song; the queue the app remembers must drop it then.
#[test]
fn a_free_queued_song_leaves_the_queue_once_it_plays() {
    let mut app = app_for("free-queue-consumed", true, SONG);
    run(
        &mut app,
        Action::AddToQueue {
            uri: "spotify:track:b".into(),
            label: "B".into(),
        },
    );
    assert_eq!(app.manual_queue, vec!["spotify:track:b".to_string()]);
    // Spotify plays the queued song when the current one ends.
    app.handle_local(playing("spotify:track:b", 2));
    assert!(
        app.manual_queue.is_empty(),
        "played, so no longer owed: {:?}",
        app.manual_queue
    );
    app.backend.shutdown();
}

/// Next onto a song queued on Free consumes it too.
#[test]
fn next_onto_a_free_queued_song_consumes_it() {
    let mut app = app_for("free-queue-next", true, SONG);
    run(
        &mut app,
        Action::AddToQueue {
            uri: "spotify:track:b".into(),
            label: "B".into(),
        },
    );
    run(&mut app, Action::Next);
    app.handle_local(playing("spotify:track:b", 2));
    assert!(app.manual_queue.is_empty(), "{:?}", app.manual_queue);
    app.backend.shutdown();
}

#[test]
fn volume_drag_follows_the_handle() {
    both("volume-drag", |free, app| {
        let first = run(app, Action::PreviewVolume(30));
        let quick = run(app, Action::PreviewVolume(35));
        if free {
            // The helper hears the press at once, then at most one level
            // per interval, then the release.
            assert!(matches!(first.as_slice(), [PlayerCommand::Volume(_)]));
            assert!(quick.is_empty());
            app.volume_drag_sent = Some(Instant::now() - VOLUME_DRAG_INTERVAL);
            let later = run(app, Action::PreviewVolume(40));
            assert_eq!(later, vec![PlayerCommand::Volume(percent_to_volume(40))]);
        } else {
            // librespot previews every move.
            assert!(matches!(
                first.as_slice(),
                [PlayerCommand::VolumePreview(_)]
            ));
            assert!(matches!(
                quick.as_slice(),
                [PlayerCommand::VolumePreview(_)]
            ));
        }
        let released = run(app, Action::SetVolume(45));
        assert_eq!(released, vec![PlayerCommand::Volume(percent_to_volume(45))]);
        assert_eq!(app.now_playing().unwrap().volume_percent, 45);
    });
}
