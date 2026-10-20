use rts_base::{Command, IVec2, PlayerId, TILE, UnitId};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, MAX_COMMANDS_PER_TURN, MAX_MESSAGE_BYTES, MAX_SETUP_BYTES,
    MAX_UNITS_PER_COMMAND, MatchSetup, PROTOCOL_VERSION, PlayerInput, Replay, ServerMessage, Turn,
    decode_client, decode_server, encode_client, encode_server, validate_commands,
};

fn setup() -> MatchSetup {
    MatchSetup {
        rules: include_str!("../../../assets/data/core.rules.ron").to_owned(),
        map: r#"(name: "network", players: [(start: (0.5, 0.5)), (start: (1.5, 0.5))],
            tiles: ["....", "...."], units: [
                (kind: "worker", owner: 0, pos: (0.5, 0.5)),
                (kind: "soldier", owner: 1, pos: (1.5, 0.5)),
            ])"#
        .to_owned(),
        seed: 42,
        players: 2,
    }
}

fn turn(tick: u32, commands: Vec<Command>) -> Turn {
    Turn {
        tick,
        inputs: vec![
            PlayerInput {
                player: PlayerId(0),
                commands,
            },
            PlayerInput {
                player: PlayerId(1),
                commands: Vec::new(),
            },
        ],
    }
}

fn move_command() -> Command {
    Command::Move {
        units: vec![UnitId(0)],
        target: IVec2::new(TILE * 7 / 2, TILE / 2),
    }
}

fn stop_command() -> Command {
    Command::Stop {
        units: vec![UnitId(0)],
    }
}

#[test]
fn all_wire_messages_roundtrip() {
    for message in [
        ClientMessage::Create {
            version: PROTOCOL_VERSION,
            checksum: setup().checksum(),
            players: 2,
        },
        ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: "AB12C3".to_owned(),
            checksum: 99,
        },
        ClientMessage::Start,
        ClientMessage::Input {
            tick: 2,
            commands: vec![move_command(), stop_command()],
        },
        ClientMessage::Hash {
            tick: 1,
            hash: u64::MAX,
        },
        ClientMessage::Leave,
    ] {
        assert_eq!(
            decode_client(&encode_client(&message).unwrap()).unwrap(),
            message
        );
    }
    for message in [
        ServerMessage::Lobby {
            code: "AB12C3".to_owned(),
            player: PlayerId(1),
            players: 2,
            capacity: 2,
        },
        ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
        },
        ServerMessage::Turn(turn(1, vec![move_command(), stop_command()])),
        ServerMessage::HashConfirmed {
            tick: 1,
            hash: u64::MAX,
        },
        ServerMessage::Ended {
            reason: "disconnected".to_owned(),
        },
        ServerMessage::Error {
            message: "room full".to_owned(),
        },
    ] {
        assert_eq!(
            decode_server(&encode_server(&message).unwrap()).unwrap(),
            message
        );
    }
}

#[test]
fn setup_checksum_uses_fixed_width_encoding_and_all_setup_fields() {
    let original = setup();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(original.rules.len() as u64).to_le_bytes());
    bytes.extend_from_slice(original.rules.as_bytes());
    bytes.extend_from_slice(&(original.map.len() as u64).to_le_bytes());
    bytes.extend_from_slice(original.map.as_bytes());
    bytes.extend_from_slice(&original.seed.to_le_bytes());
    bytes.push(original.players);
    let reference = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    assert_eq!(original.checksum(), reference);
    for index in 0..4 {
        let mut changed = original.clone();
        match index {
            0 => changed.rules.push(' '),
            1 => changed.map.push(' '),
            2 => changed.seed += 1,
            _ => changed.players = 1,
        }
        assert_ne!(original.checksum(), changed.checksum());
    }
    let mut left = original.clone();
    let mut right = original;
    left.rules = "a".to_owned();
    left.map = "bc".to_owned();
    right.rules = "ab".to_owned();
    right.map = "c".to_owned();
    assert_ne!(left.checksum(), right.checksum());
}

#[test]
fn turn_commands_execute_once_at_agreed_tick_and_preserve_order() {
    let mut sim = setup().simulation().unwrap();
    let initial = sim.unit(UnitId(0)).unwrap().pos;
    turn(1, vec![stop_command(), move_command()])
        .apply(&mut sim)
        .unwrap();
    assert_eq!(sim.tick(), 1);
    assert_ne!(sim.unit(UnitId(0)).unwrap().pos, initial);
    assert!(sim.move_target(UnitId(0)).is_some());
    turn(2, vec![stop_command()]).apply(&mut sim).unwrap();
    let stopped = sim.unit(UnitId(0)).unwrap().pos;
    for tick in 3..6 {
        turn(tick, vec![]).apply(&mut sim).unwrap();
        assert_eq!(sim.unit(UnitId(0)).unwrap().pos, stopped);
        assert_eq!(sim.move_target(UnitId(0)), None);
    }
}

#[test]
fn turn_uses_full_simulation_player_count_and_rejects_bad_order_atomically() {
    let mut sim = setup().simulation().unwrap();
    let hash = sim.state_hash();
    let valid = turn(1, vec![move_command()]);
    let mut invalid = vec![turn(0, vec![]), turn(2, vec![])];
    let mut missing = valid.clone();
    missing.inputs.pop();
    invalid.push(missing);
    let mut reversed = valid.clone();
    reversed.inputs.reverse();
    invalid.push(reversed);
    let mut duplicate = valid.clone();
    duplicate.inputs[1].player = PlayerId(0);
    invalid.push(duplicate);
    let mut outside = valid.clone();
    outside.inputs[1].player = PlayerId(2);
    invalid.push(outside);
    let mut extra = valid.clone();
    extra.inputs.push(PlayerInput {
        player: PlayerId(2),
        commands: vec![],
    });
    invalid.push(extra);
    for bad in invalid {
        assert!(bad.apply(&mut sim).is_err());
        assert_eq!(sim.state_hash(), hash);
    }
    valid.apply(&mut sim).unwrap();
    let hash = sim.state_hash();
    assert!(valid.apply(&mut sim).is_err());
    assert_eq!(sim.state_hash(), hash);
}

#[test]
fn turn_checks_command_ownership_in_the_simulation() {
    let mut sim = setup().simulation().unwrap();
    let initial = sim.clone();
    let mut hostile = turn(
        1,
        vec![Command::Move {
            units: vec![UnitId(1), UnitId(u32::MAX)],
            target: IVec2::new(TILE / 2, TILE * 3 / 2),
        }],
    );
    hostile.inputs[1].commands.push(move_command());
    hostile.apply(&mut sim).unwrap();
    let mut expected = initial;
    expected.step();
    assert_eq!(sim.state_hash(), expected.state_hash());
}

#[test]
fn rejects_malformed_versions_slots_codes_ticks_and_unknown_fields() {
    for data in [
        "",
        "Input(",
        "Start trailing",
        "Start(extra: 0)",
        "Create(version: 2, checksum: 0, players: 2)",
        "Create(version: 1, checksum: 0, players: 0)",
        "Create(version: 1, checksum: 0, players: 9)",
        "Join(version: 1, code: \"abc123\", checksum: 0)",
        "Join(version: 1, code: \"ABC12\", checksum: 0)",
        "Input(tick: 0, commands: [])",
        "Hash(tick: 0, hash: 0)",
        "Create(version: 1, checksum: 0, players: 2, extra: 1)",
    ] {
        assert!(decode_client(data).is_err(), "accepted {data}");
    }
    for data in [
        "Started(players: 2, input_delay: 0)",
        "Lobby(code: \"ABC123\", player: (2), players: 2, capacity: 2)",
        "Turn((tick: 1, inputs: []))",
        "HashConfirmed(tick: 0, hash: 0)",
    ] {
        assert!(decode_server(data).is_err(), "accepted {data}");
    }
}

#[test]
fn enforces_command_unit_and_frame_limits_on_encode_and_decode() {
    assert!(validate_commands(&vec![stop_command(); MAX_COMMANDS_PER_TURN]).is_ok());
    assert!(
        validate_commands(&[Command::Stop {
            units: vec![UnitId(0); MAX_UNITS_PER_COMMAND]
        }])
        .is_ok()
    );
    for commands in [
        vec![stop_command(); MAX_COMMANDS_PER_TURN + 1],
        vec![Command::Stop {
            units: vec![UnitId(0); MAX_UNITS_PER_COMMAND + 1],
        }],
    ] {
        let message = ClientMessage::Input { tick: 2, commands };
        assert!(encode_client(&message).is_err());
        assert!(decode_client(&ron::to_string(&message).unwrap()).is_err());
        if let ClientMessage::Input { commands, .. } = message {
            assert!(
                turn(1, commands)
                    .apply(&mut setup().simulation().unwrap())
                    .is_err()
            );
        }
    }
    assert!(decode_client(&" ".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
    assert!(decode_server(&" ".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
    let large = ServerMessage::Error {
        message: "x".repeat(MAX_MESSAGE_BYTES),
    };
    assert!(encode_server(&large).is_err());
    let large = ClientMessage::Input {
        tick: 2,
        commands: vec![
            Command::Stop {
                units: vec![UnitId(u32::MAX); MAX_UNITS_PER_COMMAND]
            };
            MAX_COMMANDS_PER_TURN
        ],
    };
    assert!(encode_client(&large).is_err());
}

#[test]
fn setup_rejects_malformed_oversized_invalid_and_different_slot_count() {
    for index in 0..6 {
        let mut invalid = setup();
        match index {
            0 => invalid.rules = "malformed".to_owned(),
            1 => invalid.map = "malformed".to_owned(),
            2 => invalid.players = 1,
            3 => invalid.players = 0,
            4 => invalid.map = " ".repeat(MAX_SETUP_BYTES + 1),
            _ => invalid.rules = " ".repeat(MAX_SETUP_BYTES + 1),
        }
        assert!(invalid.simulation().is_err());
        assert!(Replay::new(invalid).is_err());
    }
}

#[test]
fn replay_moves_stops_roundtrips_and_reproduces_every_hash() {
    let setup = setup();
    let mut sim = setup.simulation().unwrap();
    let mut replay = Replay::new(setup).unwrap();
    assert_eq!(replay.playback().unwrap().state_hash(), sim.state_hash());
    for tick in 1..=10 {
        let commands = match tick {
            1 => vec![move_command()],
            7 => vec![stop_command()],
            _ => vec![],
        };
        let turn = turn(tick, commands);
        turn.apply(&mut sim).unwrap();
        replay.push(turn, sim.state_hash()).unwrap();
        assert_eq!(replay.playback().unwrap().state_hash(), sim.state_hash());
    }
    let text = replay.to_ron().unwrap();
    let loaded = Replay::from_ron(&text).unwrap();
    assert_eq!(loaded, replay);
    assert_eq!(loaded.setup.map, replay.setup.map);
    assert_eq!(loaded.playback().unwrap().state_hash(), sim.state_hash());
}

#[test]
fn replay_rejects_invalid_continuity_versions_players_and_corruption() {
    let mut replay = Replay::new(setup()).unwrap();
    assert!(replay.push(turn(2, vec![]), 0).is_err());
    assert_eq!(replay.turns, []);
    let mut sim = replay.setup.simulation().unwrap();
    let first = turn(1, vec![move_command()]);
    first.apply(&mut sim).unwrap();
    replay.push(first.clone(), sim.state_hash()).unwrap();
    assert!(replay.push(first, sim.state_hash()).is_err());
    assert!(replay.push(turn(3, vec![]), sim.state_hash()).is_err());
    for index in 0..7 {
        let mut bad = replay.clone();
        match index {
            0 => bad.version += 1,
            1 => bad.final_hash ^= 1,
            2 => bad.turns[0].tick = 2,
            3 => {
                bad.turns[0].inputs.pop();
            }
            4 => bad.turns[0].inputs.reverse(),
            5 => bad.turns[0].inputs[0].commands.clear(),
            _ => bad.setup.map = bad.setup.map.replace("0.5, 0.5", "0.75, 0.5"),
        }
        assert!(bad.playback().is_err(), "accepted corruption {index}");
        assert!(bad.to_ron().is_err());
        assert!(Replay::from_ron(&ron::to_string(&bad).unwrap()).is_err());
    }
    assert!(Replay::from_ron("malformed").is_err());
    assert!(Replay::from_ron(&" ".repeat(rts_net::MAX_REPLAY_BYTES + 1)).is_err());
}
