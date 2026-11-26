use rts_base::{Command, DepositId, IVec2, PlayerId, TILE, UnitId};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, LobbySetup, MAX_COMMANDS_PER_TURN, MAX_MESSAGE_BYTES,
    MAX_SETUP_BYTES, MAX_UNITS_PER_COMMAND, MatchSetup, PROTOCOL_VERSION, PlayerInput, Replay,
    ServerMessage, Turn, decode_client, decode_server, encode_client, encode_server,
    validate_commands,
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

fn lobby() -> LobbySetup {
    LobbySetup {
        rules: setup().rules,
        map: "Network".to_owned(),
        players: 2,
    }
}

const fn intro() -> rts_base::ViewIntro {
    rts_base::ViewIntro {
        player: PlayerId(1),
        players: 2,
        size: IVec2::new(4, 2),
        start: IVec2::new(TILE / 2, TILE / 2),
        skirmish: false,
        fog_disabled: false,
    }
}

fn frame() -> rts_base::ViewFrame {
    let sim = setup().simulation().unwrap();
    rts_base::PlayerKnowledge::new(&sim, PlayerId(1)).observe(&sim)
}

#[test]
fn start_frame_roundtrips_server_fog_setting() {
    for fog_disabled in [false, true] {
        let message = ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
            view: rts_base::ViewIntro {
                fog_disabled,
                ..intro()
            },
        };
        assert_eq!(
            decode_server(&encode_server(&message).unwrap()).unwrap(),
            message
        );
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
#[expect(
    clippy::too_many_lines,
    reason = "covers every client and server wire variant in one round-trip check"
)]
fn all_wire_messages_roundtrip_and_stay_compact() {
    for message in [
        ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
        ClientMessage::Ready { ready: true },
        ClientMessage::Ready { ready: false },
        ClientMessage::Create {
            version: PROTOCOL_VERSION,
            name: "Host".to_owned(),
            public: true,
        },
        ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: "AB12C3".to_owned(),
            name: "Guest".to_owned(),
        },
        ClientMessage::Loaded {
            checksum: lobby().checksum(),
        },
        ClientMessage::Start,
        ClientMessage::Input {
            tick: 2,
            commands: vec![move_command(), stop_command()],
        },
        ClientMessage::Input {
            tick: 3,
            commands: vec![
                Command::Gather {
                    units: vec![UnitId(0)],
                    deposit: DepositId(2),
                },
                Command::BuildBarracks {
                    units: vec![UnitId(0)],
                    target: IVec2::new(TILE, TILE),
                },
                Command::TrainWorker {
                    units: vec![UnitId(1)],
                },
                Command::TrainRifleSquad {
                    units: vec![UnitId(2)],
                },
                Command::TrainScout {
                    units: vec![UnitId(2)],
                },
            ],
        },
        ClientMessage::Leave,
    ] {
        let encoded = encode_client(&message).unwrap();
        let ron_len = ron::to_string(&message).unwrap().len();
        assert!(
            encoded.len() < ron_len,
            "binary ({}) should be smaller than RON ({ron_len}) for {message:?}",
            encoded.len()
        );
        assert_eq!(decode_client(&encoded).unwrap(), message);
    }

    let sample_frame = frame();
    let frame_bytes = encode_server(&ServerMessage::View {
        frame: sample_frame.clone(),
    })
    .unwrap();
    let frame_ron_len = ron::to_string(&ServerMessage::View {
        frame: sample_frame.clone(),
    })
    .unwrap()
    .len();
    assert!(
        frame_bytes.len() * 4 < frame_ron_len,
        "binary view frame ({}) should be at least 4x smaller than RON ({frame_ron_len})",
        frame_bytes.len()
    );

    for message in [
        ServerMessage::Rooms {
            rooms: vec![room_info()],
        },
        ServerMessage::Rooms { rooms: vec![] },
        ServerMessage::Setup(lobby()),
        ServerMessage::Lobby {
            code: "AB12C3".to_owned(),
            player: PlayerId(1),
            public: true,
            slots: vec![
                Some(rts_net::LobbySlot {
                    name: "Host".into(),
                    ready: true,
                }),
                Some(rts_net::LobbySlot {
                    name: "Guest".into(),
                    ready: false,
                }),
            ],
        },
        ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
            view: intro(),
        },
        ServerMessage::View {
            frame: sample_frame,
        },
        ServerMessage::Ended {
            reason: "disconnected".to_owned(),
        },
        ServerMessage::Error {
            message: "room full".to_owned(),
        },
    ] {
        let expected = match &message {
            ServerMessage::Setup(content) => ServerMessage::Setup(content.minified()),
            _ => message.clone(),
        };
        assert_eq!(
            decode_server(&encode_server(&message).unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn setup_checksum_uses_fixed_width_encoding_and_all_setup_fields() {
    let original = setup().minified();
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
            0 => changed.rules = changed.rules.replace("max_health:40", "max_health:41"),
            1 => changed.map = changed.map.replace("network", "other"),
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
fn rejects_malformed_truncated_trailing_and_invalid_wire_frames() {
    for data in [
        &[][..],
        &[99],
        &[0, 9],                               // Rooms with old version 9
        &[0, 10, 0],                           // trailing byte
        &[4, 2],                               // Ready with invalid bool
        &[5, 0],                               // Start with trailing byte
        &[6, 0, 0],                            // Input with tick 0
        &[0x80, 0xff, 0xff, 0xff, 0xff, 0xff], // corrupt compressed header
        b"Rooms(version:10)",
    ] {
        assert!(decode_client(data).is_err(), "accepted {data:?}");
    }
    for data in [
        &[][..],
        &[99],
        &[3, 2, 0], // Started with truncated payload
        &[4, 0],    // truncated View
        b"Turn(turn:(tick:1,inputs:[]),hash:0)",
    ] {
        assert!(decode_server(data).is_err(), "accepted {data:?}");
    }
}

#[test]
fn enforces_command_unit_and_frame_limits_on_encode_and_decode() {
    assert!(
        validate_commands(&vec![
            Command::Stop { units: vec![] };
            MAX_COMMANDS_PER_TURN
        ])
        .is_ok()
    );
    assert!(
        validate_commands(&[Command::Stop {
            units: (0..u32::try_from(MAX_UNITS_PER_COMMAND).unwrap())
                .map(UnitId)
                .collect()
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
        if let ClientMessage::Input { commands, .. } = message {
            assert!(
                turn(1, commands)
                    .apply(&mut setup().simulation().unwrap())
                    .is_err()
            );
        }
    }
    assert!(decode_client(&vec![0; MAX_MESSAGE_BYTES + 1]).is_err());
    assert!(decode_server(&vec![6; MAX_MESSAGE_BYTES + 1]).is_err());
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
    let mut replay = Replay::new(setup).unwrap();
    let mut sim = replay.simulation().unwrap();
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
    let mut sim = replay.simulation().unwrap();
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

#[test]
fn setup_messages_minify_and_compress_large_rules_and_validate_content() {
    let mut content = lobby();
    content.rules.push_str("\n//");
    content
        .rules
        .push_str(&"\"\\".repeat((MAX_SETUP_BYTES - content.rules.len()) / 2));
    content
        .rules
        .push_str(&" ".repeat(MAX_SETUP_BYTES - content.rules.len()));
    let message = ServerMessage::Setup(content.clone());
    let encoded = encode_server(&message).unwrap();
    assert!(encoded.len() < content.rules.len());
    assert!(encoded.len() < rts_net::MAX_SERVER_MESSAGE_BYTES);
    assert_eq!(
        decode_server(&encoded).unwrap(),
        ServerMessage::Setup(content.minified())
    );
    assert_eq!(content.checksum(), content.minified().checksum());
    for invalid in [
        LobbySetup {
            players: 0,
            ..content.clone()
        },
        LobbySetup {
            map: "bad\nname".to_owned(),
            ..content.clone()
        },
        LobbySetup {
            rules: "invalid".to_owned(),
            ..content.clone()
        },
        LobbySetup {
            rules: " ".repeat(MAX_SETUP_BYTES + 1),
            ..content
        },
    ] {
        let message = ServerMessage::Setup(invalid);
        assert!(encode_server(&message).is_err());
    }
    assert!(decode_server(&vec![0; rts_net::MAX_SERVER_MESSAGE_BYTES + 1]).is_err());
}

#[test]
fn lobby_setup_reveals_no_map_content_and_checks_every_field() {
    let content = lobby();
    let ServerMessage::Setup(decoded) =
        decode_server(&encode_server(&ServerMessage::Setup(content.clone())).unwrap()).unwrap()
    else {
        panic!("expected setup")
    };
    assert_eq!(decoded, content.minified());
    let map = setup().map;
    assert!(!decoded.rules.contains("network") && !map.contains("Network"));
    for index in 0..3 {
        let mut changed = content.clone();
        match index {
            0 => changed.rules = changed.rules.replace("max_health: 40", "max_health: 41"),
            1 => changed.map = "Other".to_owned(),
            _ => changed.players = 1,
        }
        assert_ne!(content.checksum(), changed.checksum());
    }
}

#[test]
fn view_messages_are_bounded_and_large_input_still_fits() {
    let commands = vec![Command::Stop {
        units: (0..u32::try_from(MAX_UNITS_PER_COMMAND).unwrap())
            .map(|offset| UnitId(u32::MAX - offset))
            .collect(),
    }];
    let client = ClientMessage::Input {
        tick: u32::MAX,
        commands,
    };
    let client_bytes = encode_client(&client).unwrap();
    assert!(client_bytes.len() < MAX_MESSAGE_BYTES);
    assert_eq!(decode_client(&client_bytes).unwrap(), client);

    let mut large = frame();
    let unit = large.units[0].clone();
    large.units.extend(std::iter::repeat_n(unit, 1024));
    let message = ServerMessage::View { frame: large };
    let encoded = encode_server(&message).unwrap();
    assert!(encoded.len() <= rts_net::MAX_VIEW_MESSAGE_BYTES);
    assert!(
        encoded.len() < 4096,
        "1025 units compressed cleanly to {} bytes",
        encoded.len()
    );
    assert_eq!(decode_server(&encoded).unwrap(), message);

    let mut oversized = encoded;
    oversized.resize(rts_net::MAX_VIEW_MESSAGE_BYTES + 1, 0);
    assert!(decode_server(&oversized).is_err());

    for view in [
        rts_base::ViewIntro {
            players: 3,
            ..intro()
        },
        rts_base::ViewIntro {
            player: PlayerId(2),
            ..intro()
        },
    ] {
        assert!(
            encode_server(&ServerMessage::Started {
                players: 2,
                input_delay: INPUT_DELAY_TICKS,
                view,
            })
            .is_err()
        );
    }
}

#[test]
fn command_work_bounds_reject_duplicate_selections_and_repeated_orders() {
    use rts_net::{MAX_ORDERS_PER_UNIT_PER_TURN, MAX_UNIT_SELECTIONS_PER_TURN};

    let duplicate = vec![UnitId(0), UnitId(0)];
    for command in [
        Command::Stop {
            units: duplicate.clone(),
        },
        Command::Move {
            units: duplicate,
            target: IVec2::ZERO,
        },
    ] {
        let message = ClientMessage::Input {
            tick: 1,
            commands: vec![command],
        };
        assert!(encode_client(&message).is_err());
    }
    let allowed = vec![stop_command(); MAX_ORDERS_PER_UNIT_PER_TURN];
    assert!(validate_commands(&allowed).is_ok());
    let rejected = vec![stop_command(); MAX_ORDERS_PER_UNIT_PER_TURN + 1];
    assert!(validate_commands(&rejected).is_err());
    assert!(validate_commands(&[move_command(), stop_command()]).is_ok());
    let units: Vec<_> = (0..u32::try_from(MAX_UNIT_SELECTIONS_PER_TURN / 2).unwrap())
        .map(UnitId)
        .collect();
    let allowed = vec![Command::Stop { units }; 2];
    assert!(validate_commands(&allowed).is_ok());
    let mut rejected = allowed;
    rejected.push(Command::Stop {
        units: vec![UnitId(u32::MAX)],
    });
    assert!(validate_commands(&rejected).is_err());
    for commands in [
        vec![stop_command(); MAX_ORDERS_PER_UNIT_PER_TURN + 1],
        rejected,
    ] {
        let mut sim = setup().simulation().unwrap();
        let initial = sim.state_hash();
        let turn = turn(1, commands);
        assert!(turn.apply(&mut sim).is_err());
        assert_eq!(sim.state_hash(), initial);
        let message = ClientMessage::Input {
            tick: 1,
            commands: turn.inputs[0].commands.clone(),
        };
        assert!(encode_client(&message).is_err());
    }
}

#[test]
fn minified_compressed_setup_preserves_literals_checksum_and_simulation() {
    let mut original = setup();
    original.rules = original.rules.replace("speed: 2.5", "speed: 2.500000");
    original
        .rules
        .push_str("\n/* outer /* nested */ comment */\n");
    let content = LobbySetup {
        rules: original.rules.clone(),
        map: "Mod // keep /* literals */ \"quoted\" 🚀".to_owned(),
        players: 2,
    };
    let compact = content.minified();
    assert!(compact.rules.contains("speed:2.500000"));
    assert!(compact.rules.len() < original.rules.len());
    assert_eq!(compact.checksum(), content.checksum());
    let encoded = encode_server(&ServerMessage::Setup(content)).unwrap();
    assert!(encoded.len() < compact.rules.len());
    let ServerMessage::Setup(decoded) = decode_server(&encoded).unwrap() else {
        panic!("expected setup")
    };
    assert_eq!(decoded, compact);
    assert_eq!(decoded.map, "Mod // keep /* literals */ \"quoted\" 🚀");
    let mut source_sim = original.simulation().unwrap();
    let mut received_sim = MatchSetup {
        rules: decoded.rules,
        ..original
    }
    .simulation()
    .unwrap();
    for tick in 1..=20 {
        let turn = turn(
            tick,
            if tick == 1 {
                vec![move_command()]
            } else {
                vec![]
            },
        );
        turn.apply(&mut source_sim).unwrap();
        turn.apply(&mut received_sim).unwrap();
        assert_eq!(
            received_sim.state_hash(),
            source_sim.state_hash(),
            "tick {tick}"
        );
    }
}

#[test]
fn setup_checksum_ignores_comments_and_formatting_but_preserves_literal_spelling() {
    let original = setup();
    let mut formatted = original.clone();
    formatted.rules.push_str("\n// irrelevant comment\n");
    formatted.map.insert_str(0, "/* comment */\n");
    assert_eq!(original.checksum(), formatted.checksum());
    assert_eq!(original.checksum(), original.minified().checksum());
    let mut different_literal = original.clone();
    different_literal.rules = different_literal.rules.replace("speed: 2.5", "speed: 2.50");
    assert_ne!(different_literal.checksum(), original.checksum());
    assert_eq!(
        different_literal.simulation().unwrap().state_hash(),
        original.simulation().unwrap().state_hash()
    );
}

#[test]
fn binary_setup_rejects_truncation_corruption_and_trailing_bytes() {
    let valid = encode_server(&ServerMessage::Setup(lobby())).unwrap();
    for end in 0..valid.len() {
        assert!(
            decode_server(&valid[..end]).is_err(),
            "accepted truncation {end}"
        );
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(decode_server(&trailing).is_err());
    let mut corrupt = valid;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(decode_server(&corrupt).is_err());
}

fn room_info() -> rts_net::RoomInfo {
    rts_net::RoomInfo {
        code: "AB12C3".into(),
        host: "Host".into(),
        map: "Proving Grounds".into(),
        players: 1,
        capacity: 2,
    }
}

fn assert_invalid_server(message: &ServerMessage) {
    assert!(encode_server(message).is_err(), "encoded {message:?}");
}

#[test]
fn names_validate_unicode_character_limits_whitespace_and_controls() {
    use rts_net::{MAX_NAME_CHARS, validate_name};
    for valid in [
        "A".to_owned(),
        "Ada Lovelace".into(),
        "界".repeat(MAX_NAME_CHARS),
        "🚀".repeat(MAX_NAME_CHARS),
    ] {
        assert!(validate_name(&valid).is_ok(), "rejected {valid:?}");
        for message in [
            ClientMessage::Create {
                version: PROTOCOL_VERSION,
                name: valid.clone(),
                public: false,
            },
            ClientMessage::Join {
                version: PROTOCOL_VERSION,
                code: "AB12C3".into(),
                name: valid,
            },
        ] {
            assert_eq!(
                decode_client(&encode_client(&message).unwrap()).unwrap(),
                message
            );
        }
    }
    for invalid in [
        String::new(),
        " ".into(),
        " Ada".into(),
        "Ada ".into(),
        "\u{2003}Ada".into(),
        "Ada\u{a0}".into(),
        "A\nB".into(),
        "A\0B".into(),
        "A\u{7f}B".into(),
        "A\u{85}B".into(),
        "界".repeat(MAX_NAME_CHARS + 1),
    ] {
        assert!(validate_name(&invalid).is_err(), "accepted {invalid:?}");
        for message in [
            ClientMessage::Create {
                version: PROTOCOL_VERSION,
                name: invalid.clone(),
                public: true,
            },
            ClientMessage::Join {
                version: PROTOCOL_VERSION,
                code: "AB12C3".into(),
                name: invalid.clone(),
            },
        ] {
            assert!(encode_client(&message).is_err());
        }
        let mut room = room_info();
        room.host = invalid.clone();
        assert_invalid_server(&ServerMessage::Rooms { rooms: vec![room] });
        assert_invalid_server(&ServerMessage::Lobby {
            code: "AB12C3".into(),
            player: PlayerId(0),
            public: true,
            slots: vec![Some(rts_net::LobbySlot {
                name: invalid,
                ready: false,
            })],
        });
    }
}

#[test]
fn room_lists_validate_all_fields_counts_and_small_message_limit() {
    use rts_net::{MAX_LISTED_ROOMS, MAX_MAP_NAME_CHARS};
    let mut room = room_info();
    room.map = "界".repeat(MAX_MAP_NAME_CHARS);
    let maximum = ServerMessage::Rooms {
        rooms: vec![room.clone(); MAX_LISTED_ROOMS],
    };
    assert_eq!(
        decode_server(&encode_server(&maximum).unwrap()).unwrap(),
        maximum
    );
    assert_invalid_server(&ServerMessage::Rooms {
        rooms: vec![room_info(); MAX_LISTED_ROOMS + 1],
    });
    for field in 0..8 {
        let mut room = room_info();
        match field {
            0 => room.code = "bad123".into(),
            1 => room.host = " Host".into(),
            2 => room.map = "x".repeat(MAX_MAP_NAME_CHARS + 1),
            3 => room.map = "Map\nName".into(),
            4 => room.players = 0,
            5 => room.players = room.capacity + 1,
            6 => room.capacity = 0,
            _ => room.capacity = u8::try_from(rts_base::MAX_PLAYERS + 1).unwrap(),
        }
        assert_invalid_server(&ServerMessage::Rooms { rooms: vec![room] });
    }
    room.players = 1;
    room.capacity = 1;
    room.map.clear();
    let message = ServerMessage::Rooms { rooms: vec![room] };
    assert_eq!(
        decode_server(&encode_server(&message).unwrap()).unwrap(),
        message
    );
}

#[test]
fn lobby_validates_host_receiver_slot_count_and_names() {
    let member = Some(rts_net::LobbySlot {
        name: "Player".into(),
        ready: true,
    });
    let valid = ServerMessage::Lobby {
        code: "AB12C3".into(),
        player: PlayerId(2),
        public: false,
        slots: vec![member.clone(), None, member.clone()],
    };
    assert_eq!(
        decode_server(&encode_server(&valid).unwrap()).unwrap(),
        valid
    );
    for (player, slots) in [
        (0, vec![]),
        (0, vec![member.clone(); rts_base::MAX_PLAYERS + 1]),
        (1, vec![member.clone()]),
        (0, vec![None]),
        (1, vec![member.clone(), None]),
        (1, vec![None, member]),
    ] {
        assert_invalid_server(&ServerMessage::Lobby {
            code: "AB12C3".into(),
            player: PlayerId(player),
            public: true,
            slots,
        });
    }
}

#[test]
fn older_protocol_versions_are_rejected_on_live_wire() {
    for old_version in [7_u8, 8, 9, 10] {
        assert!(decode_client(&[0, old_version]).is_err());
    }
}

#[test]
fn an_original_v7_sandbox_replay_still_verifies_and_can_continue() {
    let original = concat!(
        r#"(version:7,setup:("#,
        r#"rules:"(tile_capacity:4,terrain:[(id:\"grass\",symbol:\'.\',speed_modifier:1,color:\"green\")],"#,
        r#"units:[(id:\"worker\",sprite:\"worker.png\",size:1,max_health:10,radius:0.25,speed:2)])","#,
        r#"map:"(name:\"legacy\",players:[(start:(0.5,0.5))],tiles:[\"....\"],"#,
        r#"units:[(kind:\"worker\",owner:0,pos:(0.5,0.5))])",seed:42,players:1),"#,
        r#"turns:[(tick:1,inputs:[(player:(0),commands:[Move(units:[(0)],target:(3584,512))])])],"#,
        "final_hash:8879068771994757554)",
    );
    let (mut replay, mut sim) = Replay::from_ron_with_sim(original).unwrap();
    assert_eq!(sim.tick(), 1);
    assert!(!sim.is_skirmish());

    let next = Turn {
        tick: 2,
        inputs: vec![PlayerInput {
            player: PlayerId(0),
            commands: vec![stop_command()],
        }],
    };
    next.apply(&mut sim).unwrap();
    replay.push(next, sim.state_hash()).unwrap();
    assert_eq!(replay.playback().unwrap().state_hash(), sim.state_hash());
}
