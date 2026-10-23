use bevy::{
    input_focus::{InputFocus, InputFocusVisible},
    scene::ScenePlugin,
    state::app::StatesPlugin,
    window::WindowResolution,
};
use rts_assets::Graphics;
use rts_net::{LobbySlot, MatchSetup, Replay};

use super::*;
use crate::{
    crash::CrashReport,
    game::{AppState, CatalogMap},
    storage::tests::temporary_store,
};

fn setup(map: &str) -> MatchSetup {
    let map = match map {
        "crossroads" => include_str!("../../../../assets/maps/crossroads.map.ron"),
        _ => include_str!("../../../../assets/maps/proving_grounds.map.ron"),
    };
    let players = u8::try_from(rts_base::MapData::from_ron(map).unwrap().players.len()).unwrap();
    MatchSetup {
        rules: include_str!("../../../../assets/data/core.rules.ron").into(),
        map: map.into(),
        seed: 0,
        players,
    }
}

fn catalog() -> MapCatalog {
    MapCatalog(
        [
            ("Crossroads", "crossroads"),
            ("Proving Grounds", "proving_grounds"),
        ]
        .into_iter()
        .map(|(name, file)| CatalogMap {
            name: name.into(),
            setup: setup(file),
            preview: Handle::default(),
        })
        .collect(),
    )
}

fn app(width: u32) -> App {
    app_with(width, None)
}

fn app_with(width: u32, crash: Option<CrashReport>) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        AssetPlugin::default(),
        ScenePlugin,
    ))
    .init_asset::<Image>()
    .init_asset::<Font>()
    .insert_state(AppState::Menu)
    .insert_resource(catalog())
    .insert_resource(Library::new(temporary_store("menu")))
    .insert_resource(crash.unwrap_or_default())
    .insert_resource(
        Graphics::from_ron(include_str!("../../../../assets/data/client.graphics.ron")).unwrap(),
    )
    .init_resource::<Settings>()
    .init_resource::<SessionStatus>()
    .init_resource::<UiScale>()
    .init_resource::<InputFocus>()
    .init_resource::<InputFocusVisible>()
    .init_resource::<ButtonInput<KeyCode>>()
    .add_message::<SessionAction>()
    .add_plugins(MenuPlugin);
    let mut window = Window {
        resolution: WindowResolution::new(width, 720),
        ..default()
    };
    window.focused = true;
    app.world_mut().spawn((window, PrimaryWindow));
    settle(&mut app);
    app
}

fn settle(app: &mut App) {
    for _ in 0..4 {
        app.update();
    }
}

fn act(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    settle(app);
}

fn texts(app: &mut App) -> Vec<String> {
    app.world_mut()
        .query::<&Text>()
        .iter(app.world())
        .map(|text| text.0.clone())
        .collect()
}

#[track_caller]
fn assert_shows(app: &mut App, expected: &[&str]) {
    let texts = texts(app);
    for wanted in expected {
        assert!(
            texts.iter().any(|text| text.contains(wanted)),
            "missing {wanted:?} in {texts:?}"
        );
    }
    // One page at a time, and nothing left over from earlier pages.
    let pages = app
        .world_mut()
        .query_filtered::<&Children, With<PageHost>>()
        .single(app.world())
        .unwrap()
        .len();
    assert_eq!(pages, 1);
    let roots = app
        .world_mut()
        .query_filtered::<Entity, (With<Node>, Without<ChildOf>)>()
        .iter(app.world())
        .count();
    assert_eq!(roots, 1, "only the menu root is a top-level node");
}

fn status(app: &mut App) -> Mut<'_, SessionStatus> {
    app.world_mut().resource_mut::<SessionStatus>()
}

fn sent(app: &mut App) -> Vec<SessionAction> {
    app.world_mut()
        .resource_mut::<Messages<SessionAction>>()
        .drain()
        .collect()
}

fn lobby(ready: bool) -> Lobby {
    Lobby {
        code: "AAAAAA".into(),
        player: 1,
        public: false,
        slots: vec![
            Some(LobbySlot {
                name: "Ada".into(),
                ready: true,
            }),
            Some(LobbySlot {
                name: "Grace".into(),
                ready,
            }),
        ],
    }
}

#[test]
fn a_stored_crash_report_is_offered_for_export() {
    let report = "RTS crash report\n\
                  \n\
                  Time: 2026-10-13 00:00:00 UTC\n\
                  Panic: index out of bounds\n\
                  Location: src/game.rs:112:5\n";
    let mut app = app_with(1280, Some(CrashReport::for_test(report)));
    assert_shows(
        &mut app,
        &[
            "The game closed unexpectedly",
            "Time: 2026-10-13 00:00:00 UTC",
            "Panic: index out of bounds",
            "Export report",
            "Discard",
        ],
    );
    act(&mut app, Action::DiscardCrash);
    assert!(app.world().resource::<Menu>().dialog.is_none());
    assert!(app.world().resource::<CrashReport>().pending().is_none());
    assert_shows(&mut app, &["Your next move"]);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one walk through every page and dialog"
)]
fn every_page_and_dialog_spawns_on_wide_and_compact_screens() {
    for (width, compact) in [(1280, false), (600, true)] {
        let mut app = app(width);
        assert_eq!(app.world().resource::<Menu>().compact, compact);
        assert_shows(&mut app, &["Your next move", "New game"]);
        assert_shows(
            &mut app,
            if compact {
                &["Solo", "Online", "Back"]
            } else {
                &["Singleplayer", "Multiplayer"]
            },
        );

        act(&mut app, Action::OpenNewGame);
        assert_shows(
            &mut app,
            &["Select map", "Crossroads", "4 players · 40×40 tiles"],
        );
        act(&mut app, Action::SelectMap(1));
        assert_shows(&mut app, &["Selected: Proving Grounds"]);
        act(&mut app, Action::StartGame);
        // Sent messages are drained by the next frame's message update, so check right away.

        let replay = Replay::new(setup("crossroads")).unwrap();
        app.world_mut()
            .resource_mut::<Library>()
            .add(crate::library::Shelf::Saves, &replay)
            .unwrap();
        act(&mut app, Action::Back);
        act(&mut app, Action::OpenLoadGame);
        assert_shows(
            &mut app,
            &["Load game", "Save 01", "Crossroads · 0:00", "Load save"],
        );
        act(
            &mut app,
            Action::ToggleDropdown(Dropdown::Map(Shelf::Saves)),
        );
        assert_shows(&mut app, &["All maps"]);
        act(&mut app, Action::Delete(Shelf::Saves));
        assert_shows(&mut app, &["Delete Save 01?", "This cannot be undone."]);
        act(&mut app, Action::CloseDialog);

        act(&mut app, Action::Section(Section::Multiplayer));
        assert_shows(&mut app, &["Connect to a server", "Server address"]);
        status(&mut app).server = Server::Failed {
            url: "ws://down".into(),
            reason: "Connection refused.".into(),
        };
        settle(&mut app);
        assert_shows(
            &mut app,
            &["Could not connect to ws://down. Connection refused."],
        );
        status(&mut app).server = Server::Connecting {
            url: "ws://up".into(),
        };
        settle(&mut app);
        assert_shows(&mut app, &["Connecting…", "ws://up"]);
        {
            let mut status = status(&mut app);
            status.server = Server::Connected {
                url: "ws://up:9001".into(),
            };
            status.rooms = vec![RoomInfo {
                code: "BBBBBB".into(),
                host: "Alpha".into(),
                map: "Crossroads".into(),
                players: 1,
                capacity: 4,
            }];
        }
        settle(&mut app);
        assert_shows(
            &mut app,
            &[
                "Server: up:9001",
                "Alpha's room",
                "1/4 players · Crossroads",
                "Join room",
            ],
        );
        for (dialog, title) in [
            (Action::OpenJoinByCode, "Join room with invite code"),
            (Action::OpenCreateRoom, "Create room"),
        ] {
            act(&mut app, dialog);
            assert_shows(&mut app, &[title]);
            act(&mut app, Action::Back);
            assert!(app.world().resource::<Menu>().dialog.is_none());
        }
        status(&mut app).lobby = Some(lobby(false));
        settle(&mut app);
        assert_shows(
            &mut app,
            &[
                "Room: Ada's room",
                "Invite AAAAAA",
                "Ada (host)",
                "Grace (you)",
                "Not ready",
            ],
        );
        act(&mut app, Action::LeaveRoom);
        assert_shows(&mut app, &["Leave this room?", "You can rejoin"]);
        act(&mut app, Action::CloseDialog);

        status(&mut app).has_replay = true;
        act(&mut app, Action::Section(Section::Replays));
        assert_shows(&mut app, &["Replay browser", "Last match", "Watch replay"]);
        act(&mut app, Action::Section(Section::Settings));
        assert_shows(&mut app, &["Player name", "Interface scale", "125%"]);
    }
}

#[test]
fn actions_follow_session_state_and_send_session_actions() {
    let mut app = app(1280);
    let _ = sent(&mut app);

    act(&mut app, Action::OpenNewGame);
    act(&mut app, Action::SelectMap(1));
    app.world_mut().write_message(Action::StartGame);
    app.update();
    assert_eq!(sent(&mut app), [SessionAction::NewGame { map: 1 }]);

    // Joining needs a connection and a complete invite code.
    act(&mut app, Action::Section(Section::Multiplayer));
    app.world_mut().resource_mut::<MenuForm>().code = "ABC12".into();
    app.world_mut().write_message(Action::JoinByCode);
    app.update();
    assert_eq!(sent(&mut app), []);
    status(&mut app).server = Server::Connected {
        url: "ws://up".into(),
    };
    app.world_mut().resource_mut::<MenuForm>().code = "ABC123".into();
    app.world_mut().resource_mut::<Settings>().name = "Ada".into();
    app.world_mut().write_message(Action::JoinByCode);
    app.update();
    assert_eq!(
        sent(&mut app),
        [SessionAction::JoinRoom {
            name: "Ada".into(),
            code: "ABC123".into()
        }]
    );

    // In a room: Ready toggles, Start waits for everyone, and Back asks before leaving.
    status(&mut app).lobby = Some(lobby(false));
    settle(&mut app);
    app.world_mut().write_message(Action::ToggleReady);
    app.world_mut().write_message(Action::StartMatch);
    app.update();
    assert_eq!(sent(&mut app), [SessionAction::SetReady(true)]);
    app.world_mut().write_message(Action::Back);
    app.update();
    assert_eq!(
        app.world().resource::<Menu>().dialog,
        Some(Dialog::Confirm(Confirm::LeaveRoom))
    );
    app.world_mut().write_message(Action::Confirm);
    app.update();
    assert_eq!(sent(&mut app), [SessionAction::LeaveRoom]);

    // A lobby or a lost connection closes the join and create dialogs.
    status(&mut app).lobby = None;
    act(&mut app, Action::OpenCreateRoom);
    status(&mut app).server = Server::Offline;
    settle(&mut app);
    assert!(app.world().resource::<Menu>().dialog.is_none());
}

#[test]
fn successful_connections_become_the_recent_server() {
    let mut app = app(1280);
    status(&mut app).server = Server::Connected {
        url: "ws://10.0.0.2:9001".into(),
    };
    settle(&mut app);
    assert_eq!(
        app.world().resource::<Settings>().recent_server.as_deref(),
        Some("ws://10.0.0.2:9001")
    );
    status(&mut app).server = Server::Offline;
    act(&mut app, Action::Section(Section::Multiplayer));
    act(&mut app, Action::UseRecentServer);
    assert_eq!(app.world().resource::<MenuForm>().url, "ws://10.0.0.2:9001");
    let field = app
        .world_mut()
        .query::<(&Field, &EditableText)>()
        .iter(app.world())
        .find(|(field, _)| **field == Field::Url)
        .map(|(_, text)| text.value().to_string());
    assert_eq!(field.as_deref(), Some("ws://10.0.0.2:9001"));
}

#[test]
fn leaving_a_live_match_asks_first() {
    let mut app = app(1280);
    app.world_mut().resource_mut::<PauseMenu>().open = true;
    let _ = sent(&mut app);
    {
        let mut status = status(&mut app);
        status.running = true;
        status.kind = MatchKind::Solo;
    }
    app.world_mut()
        .write_message(Action::Game(SessionAction::Leave));
    app.update();
    assert_eq!(sent(&mut app), []);
    assert_eq!(
        app.world().resource::<Menu>().dialog,
        Some(Dialog::Confirm(Confirm::LeaveMatch))
    );
    app.world_mut().write_message(Action::Confirm);
    app.update();
    assert_eq!(sent(&mut app), [SessionAction::Leave]);

    // Finished replays and stopped matches leave at once.
    status(&mut app).kind = MatchKind::Playback;
    app.world_mut()
        .write_message(Action::Game(SessionAction::Leave));
    app.update();
    assert_eq!(sent(&mut app), [SessionAction::Leave]);
}

#[test]
fn browse_filters_orders_and_keeps_a_valid_selection() {
    let record = |id: &str, map: &str, saved_at| Record {
        id: id.into(),
        meta: crate::library::RecordMeta {
            name: id.into(),
            map: map.into(),
            players: 2,
            ticks: 0,
            saved_at,
            setup: setup("proving_grounds"),
        },
    };
    let records = [
        record("c", "Crossroads", 3),
        record("b", "Proving Grounds", 2),
        record("a", "Crossroads", 1),
    ];
    let mut browse = Browse::default();
    let ids = |browse: &Browse| -> Vec<String> {
        browse
            .filter(&records)
            .iter()
            .map(|record| record.id.clone())
            .collect()
    };
    assert_eq!(ids(&browse), ["c", "b", "a"]);
    browse.oldest_first = true;
    browse.map = Some("Crossroads".into());
    assert_eq!(ids(&browse), ["a", "c"]);

    select_first(&mut browse, &records, false);
    assert_eq!(browse.selected, Some(ReplayPick::Stored("a".into())));
    browse.selected = Some(ReplayPick::Stored("b".into()));
    select_first(&mut browse, &records, false);
    assert_eq!(browse.selected, Some(ReplayPick::Stored("a".into())));
    browse.selected = Some(ReplayPick::Last);
    select_first(&mut browse, &records, true);
    assert_eq!(
        browse.selected,
        Some(ReplayPick::Stored("a".into())),
        "the last match is not listed under a map filter"
    );
    browse.map = None;
    browse.selected = None;
    select_first(&mut browse, &records, true);
    assert_eq!(browse.selected, Some(ReplayPick::Last));
    select_first(&mut browse, &[], false);
    assert_eq!(browse.selected, None);
}

#[test]
fn form_validates_addresses_and_codes() {
    let mut form = MenuForm::default();
    assert!(form.url_valid());
    for (url, valid) in [
        (" ws:// ", false),
        ("http://host", false),
        (" wss://relay.example ", true),
    ] {
        form.url = url.into();
        assert_eq!(form.url_valid(), valid, "{url}");
    }
    for (code, valid) in [("K7Q2PX", true), ("K7Q2P", false), ("k7q2px", false)] {
        form.code = code.into();
        assert_eq!(form.code_valid(), valid, "{code}");
    }
}

#[test]
fn pause_menu_escape_resume_and_leave_confirmation_preserve_selection() {
    use crate::{
        input::Selection,
        pause_menu::{PauseMenu, PauseMenuPlugin},
    };
    use rts_base::UnitId;
    use std::collections::BTreeSet;

    let mut app = app(1280);
    app.add_plugins(PauseMenuPlugin)
        .insert_resource(Selection {
            units: BTreeSet::from([UnitId(0)]),
            touch_mode: false,
        })
        .insert_resource(MoveTargeting(false));
    {
        let mut status = status(&mut app);
        status.running = true;
        status.kind = MatchKind::Solo;
        status.has_replay = true;
    }
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::InGame);
    settle(&mut app);
    let escape = |app: &mut App| {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
    };
    app.world_mut().resource_mut::<MoveTargeting>().0 = true;
    escape(&mut app);
    assert!(!app.world().resource::<MoveTargeting>().0);
    assert!(!app.world().resource::<PauseMenu>().open);
    escape(&mut app);
    assert!(app.world().resource::<PauseMenu>().open);
    assert_eq!(
        app.world().resource::<Selection>().units,
        BTreeSet::from([UnitId(0)])
    );
    let visible = texts(&mut app);
    for label in [
        "Resume",
        "Save game",
        "Save replay",
        "Leave",
        "Game paused.",
    ] {
        assert!(visible.iter().any(|text| text == label), "missing {label}");
    }
    act(&mut app, Action::Game(SessionAction::SaveGame));
    act(&mut app, Action::Game(SessionAction::SaveReplay));
    assert_eq!(
        sent(&mut app),
        [SessionAction::SaveGame, SessionAction::SaveReplay]
    );
    act(&mut app, Action::Game(SessionAction::Leave));
    assert!(app.world().resource::<Menu>().dialog.is_some());
    escape(&mut app);
    assert!(app.world().resource::<Menu>().dialog.is_none());
    assert!(app.world().resource::<PauseMenu>().open);
    escape(&mut app);
    assert!(!app.world().resource::<PauseMenu>().open);
    act(&mut app, Action::TogglePause);
    assert!(app.world().resource::<PauseMenu>().open);
    act(&mut app, Action::Resume);
    assert!(!app.world().resource::<PauseMenu>().open);
    status(&mut app).kind = MatchKind::Online;
    act(&mut app, Action::TogglePause);
    assert!(
        texts(&mut app)
            .iter()
            .any(|text| text == "Online match continues while this menu is open.")
    );
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Menu);
    settle(&mut app);
    assert!(!app.world().resource::<PauseMenu>().open);
}

#[test]
fn closed_pause_menu_clears_focus_and_ignores_hidden_actions() {
    use crate::pause_menu::PauseMenuPlugin;
    use bevy::ui_widgets::Activate;
    let mut app = app(1280);
    app.add_plugins(PauseMenuPlugin);
    status(&mut app).running = true;
    status(&mut app).kind = MatchKind::Solo;
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::InGame);
    settle(&mut app);
    act(&mut app, Action::TogglePause);
    let save = app
        .world_mut()
        .query::<(Entity, &Action)>()
        .iter(app.world())
        .find(|(_, action)| **action == Action::Game(SessionAction::SaveGame))
        .unwrap()
        .0;
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(save, bevy::input_focus::FocusCause::Navigated);
    app.world_mut().trigger(Activate { entity: save });
    settle(&mut app);
    assert_eq!(sent(&mut app), [SessionAction::SaveGame]);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(!app.world().resource::<PauseMenu>().open);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    // Simulate activation that was dispatched before focus was cleared.
    app.world_mut().trigger(Activate { entity: save });
    act(&mut app, Action::Game(SessionAction::Leave));
    assert_eq!(sent(&mut app), []);
    assert!(app.world().resource::<Menu>().dialog.is_none());
}
