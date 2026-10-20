use rts_base::{IVec2, TILE, UnitId};

use super::*;

fn setup() -> MatchSetup {
    MatchSetup {
        rules: include_str!("../../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../../assets/maps/proving_grounds.map.ron").into(),
        seed: 0,
        players: 2,
    }
}

#[test]
fn local_input_delay_and_replay_share_the_same_turns() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Offline, PlayerId(0), 0.0)
        .unwrap();
    let mut previous = PreviousPositions::default();
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    let target = IVec2::new(5 * TILE, TILE * 10);
    game.queue_command(Command::Move {
        units: vec![UnitId(0)],
        target,
    });
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.05,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_none());
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.10,
    )
    .unwrap();
    assert_eq!(game.sim.move_target(UnitId(0)), Some(target));
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.15,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_some());
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.20,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_none());
    let replay = session.replay().unwrap();
    assert_eq!(replay.turns.len(), 4);
    assert_eq!(
        replay.playback().unwrap().state_hash(),
        game.sim.state_hash()
    );
}

#[test]
fn absent_network_turn_stalls_without_consuming_input_or_repeating_interpolation() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Online, PlayerId(0), 0.0)
        .unwrap();
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    let mut previous = PreviousPositions(BTreeMap::from([(UnitId(0), IVec2::ZERO)]));
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    let hash = game.sim.state_hash();
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.1,
    )
    .unwrap();
    assert_eq!(game.sim.state_hash(), hash);
    assert_eq!(game.pending.len(), 1);
    assert!(previous.0.is_empty());
    assert!(
        advance(
            &mut game,
            &mut previous,
            &mut session,
            &mut status,
            &mut network,
            16.0
        )
        .is_err()
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn headless() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(AppState::Menu)
        .insert_resource(Time::<()>::default())
        .insert_resource(MatchData(setup()))
        .init_resource::<Selection>()
        .init_resource::<PreviousPositions>()
        .add_plugins(SessionPlugin);
    app
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn completed_replay_load_cannot_replace_a_match_starting_this_frame() {
    let mut app = headless();
    let text = Replay::new(setup()).unwrap().to_ron().unwrap();
    app.world().resource::<ReplayIo>().complete_for_test(text);
    app.world_mut().write_message(SessionAction::Solo);
    app.update();
    assert!(app.world().resource::<Session>().mode == Mode::Offline);
    assert!(app.world().resource::<Match>().accepts_orders);
    assert!(
        app.world()
            .resource::<SessionStatus>()
            .message
            .contains("Leave the current match")
    );
    app.update();
    assert_eq!(
        *app.world().resource::<State<AppState>>().get(),
        AppState::InGame
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test(flavor = "current_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one scripted real-socket match from lobby through replay verification and disconnect"
)]
async fn real_client_sessions_lockstep_record_replay_and_freeze_on_disconnect() {
    use std::time::Duration;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(rts_server::serve(listener));
    let mut host = headless();
    let mut guest = headless();
    host.world_mut()
        .write_message(SessionAction::Host { url: url.clone() });
    let mut joined = false;
    let mut started = false;
    let mut moved = false;
    let mut stopped = false;
    let mut host_hashes = BTreeMap::new();
    let mut guest_hashes = BTreeMap::new();
    let initial_hash = setup().simulation().unwrap().state_hash();
    host_hashes.insert(0, initial_hash);
    guest_hashes.insert(0, initial_hash);
    for _ in 0..1000 {
        host.update();
        guest.update();
        if !joined && let Some(code) = host.world().resource::<SessionStatus>().room_code.clone() {
            guest.world_mut().write_message(SessionAction::Join {
                url: url.clone(),
                code,
            });
            joined = true;
        }
        if !started && host.world().resource::<SessionStatus>().can_start {
            host.world_mut().write_message(SessionAction::Start);
            started = true;
        }
        for app in [&mut host, &mut guest] {
            let tick = app
                .world()
                .get_resource::<Match>()
                .map_or(0, |game| game.sim.tick());
            if tick < 40 {
                app.world_mut().run_schedule(FixedUpdate);
            }
        }
        if let Some(game) = host.world().get_resource::<Match>() {
            let tick = game.sim.tick();
            host_hashes.insert(tick, game.sim.state_hash());
            if tick >= 3 && !moved {
                host.world_mut()
                    .resource_mut::<Match>()
                    .queue_command(Command::Move {
                        units: vec![UnitId(0)],
                        target: IVec2::new(TILE * 5, TILE * 10),
                    });
                moved = true;
            }
            if tick >= 18 && !stopped {
                host.world_mut()
                    .resource_mut::<Match>()
                    .queue_command(Command::Stop {
                        units: vec![UnitId(0)],
                    });
                stopped = true;
            }
        }
        if let Some(game) = guest.world().get_resource::<Match>() {
            guest_hashes.insert(game.sim.tick(), game.sim.state_hash());
        }
        if host_hashes.contains_key(&40) && guest_hashes.contains_key(&40) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(moved && stopped);
    for tick in 0..=40 {
        assert_eq!(
            host_hashes.get(&tick),
            guest_hashes.get(&tick),
            "tick {tick}"
        );
    }
    let game = host.world().resource::<Match>();
    assert_eq!(
        game.sim.tick(),
        40,
        "{}",
        host.world().resource::<SessionStatus>().message
    );
    assert_ne!(
        game.sim.unit(UnitId(0)).unwrap().pos,
        IVec2::new(TILE * 9 / 2, TILE * 8)
    );
    let replay = host.world().resource::<Session>().replay().unwrap();
    let text = replay.to_ron().unwrap();
    assert_eq!(
        Replay::from_ron(&text)
            .unwrap()
            .playback()
            .unwrap()
            .state_hash(),
        game.sim.state_hash()
    );
    guest.world_mut().write_message(SessionAction::Leave);
    guest.update();
    for _ in 0..200 {
        host.update();
        if host.world().resource::<Session>().mode == Mode::Stopped {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(host.world().resource::<Session>().mode == Mode::Stopped);
    assert!(!host.world().resource::<Match>().accepts_orders);
    assert!(host.world().resource::<SessionStatus>().room_code.is_some());
    server.abort();
}
