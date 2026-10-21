use rts_base::{Command, IVec2, PlayerId, TILE, UnitId};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, MAX_COMMANDS_PER_TURN, MAX_MESSAGE_BYTES, MAX_SETUP_BYTES,
    MAX_UNITS_PER_COMMAND, MatchSetup, PROTOCOL_VERSION, PlayerInput, Replay, ServerFrame,
    ServerMessage, Turn, decode_client, decode_server, decode_server_binary, encode_client,
    encode_server, validate_commands,
};

fn text_frame(message: &ServerMessage) -> String {
    let ServerFrame::Text(text) = encode_server(message).unwrap() else {
        panic!("expected text frame")
    };
    text
}

fn binary_frame(message: &ServerMessage) -> Vec<u8> {
    let ServerFrame::Binary(bytes) = encode_server(message).unwrap() else {
        panic!("expected binary frame")
    };
    bytes
}

fn decode_frame(frame: &ServerFrame) -> Result<ServerMessage, rts_net::NetError> {
    match frame {
        ServerFrame::Text(text) => decode_server(text),
        ServerFrame::Binary(bytes) => decode_server_binary(bytes),
    }
}

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
            checksum: setup().checksum(),
        },
        ClientMessage::Start,
        ClientMessage::Input {
            tick: 2,
            commands: vec![move_command(), stop_command()],
        },
        ClientMessage::Leave,
    ] {
        assert_eq!(
            decode_client(&encode_client(&message).unwrap()).unwrap(),
            message
        );
    }
    for message in [
        ServerMessage::Rooms {
            rooms: vec![room_info()],
        },
        ServerMessage::Rooms { rooms: vec![] },
        ServerMessage::Setup(setup()),
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
        },
        ServerMessage::Turn {
            turn: turn(1, vec![move_command(), stop_command()]),
            hash: u64::MAX,
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
            decode_frame(&encode_server(&message).unwrap()).unwrap(),
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
        "Create(version: 1)",
        "Create(version: 2)",
        "Create(version: 3)",
        "Create(version: 2, checksum: 0)",
        "Join(version: 4, code: \"abc123\")",
        "Join(version: 4, code: \"ABC12\")",
        "Input(tick: 1, player: (0), commands: [])",
        "Input(tick: 0, commands: [])",
        "Hash(tick: 0, hash: 0)",
        "Create(version: 1, checksum: 0, players: 2, extra: 1)",
    ] {
        assert!(decode_client(data).is_err(), "accepted {data}");
    }
    for data in [
        "Started(players: 2, input_delay: 0)",
        "Lobby(code: \"ABC123\", player: (0), players: 2, capacity: 2, ready: 3)",
        "Turn(turn:(tick:1,inputs:[]),hash:0)",
        "Lobby(code: \"ABC123\", player: (2), players: 2, capacity: 2)",
        "Turn((tick: 1, inputs: []))",
        "HashConfirmed(tick: 0, hash: 0)",
    ] {
        assert!(decode_server(data).is_err(), "accepted {data}");
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

#[test]
fn setup_messages_minify_and_compress_large_documents_and_validate_content() {
    let mut content = setup();
    // Both original documents can reach their independent 4 MiB limit. Quoted and
    // backslash characters in comments expand when carried inside a RON string.
    for document in [&mut content.rules, &mut content.map] {
        document.push_str("\n//");
        document.push_str(&"\"\\".repeat((MAX_SETUP_BYTES - document.len()) / 2));
        document.push_str(&" ".repeat(MAX_SETUP_BYTES - document.len()));
    }
    let message = ServerMessage::Setup(content.clone());
    let encoded = binary_frame(&message);
    assert!(encoded.len() < content.rules.len() + content.map.len());
    assert!(encoded.len() < rts_net::MAX_SERVER_MESSAGE_BYTES);
    assert_eq!(
        decode_server_binary(&encoded).unwrap(),
        ServerMessage::Setup(content.minified())
    );
    assert_eq!(content.checksum(), content.minified().checksum());
    for invalid in [
        MatchSetup {
            players: 1,
            ..content.clone()
        },
        MatchSetup {
            map: "invalid".to_owned(),
            ..content.clone()
        },
        MatchSetup {
            rules: " ".repeat(MAX_SETUP_BYTES + 1),
            ..content
        },
    ] {
        let message = ServerMessage::Setup(invalid);
        assert!(encode_server(&message).is_err());
    }
    assert!(decode_server(&" ".repeat(rts_net::MAX_SERVER_MESSAGE_BYTES + 1)).is_err());
}

#[test]
fn turn_fan_in_accepts_every_players_independently_valid_large_input() {
    let commands = vec![Command::Stop {
        units: (0..u32::try_from(MAX_UNITS_PER_COMMAND).unwrap())
            .map(|offset| UnitId(u32::MAX - offset))
            .collect(),
    }];
    let client = ClientMessage::Input {
        tick: u32::MAX,
        commands: commands.clone(),
    };
    let client_text = encode_client(&client).unwrap();
    assert!(client_text.len() < MAX_MESSAGE_BYTES);
    assert_eq!(decode_client(&client_text).unwrap(), client);
    let message = ServerMessage::Turn {
        turn: Turn {
            tick: u32::MAX,
            inputs: (0..u8::try_from(rts_base::MAX_PLAYERS).unwrap())
                .map(|slot| PlayerInput {
                    player: PlayerId(slot),
                    commands: commands.clone(),
                })
                .collect(),
        },
        hash: u64::MAX,
    };
    let encoded = text_frame(&message);
    assert!(encoded.len() > MAX_MESSAGE_BYTES);
    assert!(encoded.len() <= rts_net::MAX_TURN_MESSAGE_BYTES);
    assert_eq!(decode_server(&encoded).unwrap(), message);
    let oversized = format!("{encoded}{}", " ".repeat(rts_net::MAX_TURN_MESSAGE_BYTES));
    assert!(decode_server(&oversized).is_err());
    let large = ServerMessage::Error {
        message: "x".repeat(MAX_MESSAGE_BYTES),
    };
    assert!(decode_server(&ron::to_string(&large).unwrap()).is_err());
    assert!(
        decode_server(&format!(
            "{}{}",
            text_frame(&ServerMessage::Started {
                players: 2,
                input_delay: INPUT_DELAY_TICKS,
            }),
            " ".repeat(MAX_MESSAGE_BYTES)
        ))
        .is_err()
    );
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
        assert!(decode_client(&ron::to_string(&message).unwrap()).is_err());
    }
    let allowed = vec![stop_command(); MAX_ORDERS_PER_UNIT_PER_TURN];
    assert!(validate_commands(&allowed).is_ok());
    let rejected = vec![stop_command(); MAX_ORDERS_PER_UNIT_PER_TURN + 1];
    assert!(validate_commands(&rejected).is_err());
    // Normal ordered replacements stay valid.
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
        assert!(decode_client(&ron::to_string(&message).unwrap()).is_err());
    }
}

#[test]
fn minified_compressed_setup_preserves_literals_checksum_and_simulation() {
    let mut original = setup();
    original.rules = original.rules.replace("speed: 2.5", "speed: 2.500000");
    original.map = original.map.replace(
        "\"network\"",
        r###"r##"Mod // keep /* literals */ \"quoted\" 🚀"##"###,
    );
    original
        .rules
        .push_str("\n/* outer /* nested */ comment */\n");
    let compact = original.minified();
    assert!(compact.rules.contains("speed:2.500000"));
    assert!(compact.rules.len() < original.rules.len());
    assert!(compact.map.contains("Mod // keep /* literals */"));
    assert_eq!(compact.checksum(), original.checksum());
    let encoded = binary_frame(&ServerMessage::Setup(original.clone()));
    assert!(encoded.len() < compact.rules.len() + compact.map.len());
    let ServerMessage::Setup(decoded) = decode_server_binary(&encoded).unwrap() else {
        panic!("expected setup")
    };
    assert_eq!(decoded, compact);
    let mut source_sim = original.simulation().unwrap();
    let mut received_sim = decoded.simulation().unwrap();
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
    let replay = Replay::new(decoded).unwrap();
    assert_eq!(Replay::from_ron(&replay.to_ron().unwrap()).unwrap(), replay);
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
fn binary_setup_rejects_forged_frames_before_client_readiness() {
    use miniz_oxide::deflate::compress_to_vec_zlib;

    let compact = setup().minified();
    let map = compress_to_vec_zlib(compact.map.as_bytes(), 6);
    let wire = |rules_len: u32, rules: &[u8]| {
        let rules = compress_to_vec_zlib(rules, 6);
        let mut bytes = b"RTSS".to_vec();
        bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        bytes.extend_from_slice(&42_u64.to_le_bytes());
        bytes.push(2);
        bytes.extend_from_slice(&rules_len.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(rules.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(compact.map.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(map.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&rules);
        bytes.extend_from_slice(&map);
        bytes
    };
    let valid = wire(
        u32::try_from(compact.rules.len()).unwrap(),
        compact.rules.as_bytes(),
    );
    assert_eq!(binary_frame(&ServerMessage::Setup(setup())), valid);
    assert_eq!(
        decode_server_binary(&valid).unwrap(),
        ServerMessage::Setup(compact.clone())
    );
    for end in 0..valid.len() {
        assert!(
            decode_server_binary(&valid[..end]).is_err(),
            "accepted truncation {end}"
        );
    }
    for offset in [17, 21, 25, 29] {
        let mut invalid = valid.clone();
        invalid[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_server_binary(&invalid).is_err());
    }
    for (offset, value) in [(0, b'?'), (4, 0), (16, 0), (16, 9)] {
        let mut invalid = valid.clone();
        invalid[offset] = value;
        assert!(decode_server_binary(&invalid).is_err());
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(decode_server_binary(&trailing).is_err());
    let mut corrupt = valid;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(decode_server_binary(&corrupt).is_err());
    assert!(decode_server_binary(&wire(0, compact.rules.as_bytes())).is_err());
    assert!(decode_server_binary(&wire(1, &[255])).is_err());
    assert!(decode_server_binary(&wire(7, b"invalid")).is_err());
    assert!(decode_server_binary(&vec![0; rts_net::MAX_SERVER_MESSAGE_BYTES + 1]).is_err());
    assert!(decode_server_binary(b"Turn(turn:(tick:1,inputs:[]),hash:0)").is_err());
    // Setup is binary-only; no legacy plaintext or base64 decoder remains.
    assert!(decode_server(&format!("Setup({})", ron::to_string(&setup()).unwrap())).is_err());
    assert!(decode_server("Setup((rules:(len:0,zlib:\"eJwDAAAAAAE=\"),map:(len:0,zlib:\"eJwDAAAAAAE=\"),seed:42,players:2))").is_err());
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
    assert!(
        decode_server(&ron::to_string(message).unwrap()).is_err(),
        "decoded {message:?}"
    );
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
            assert!(decode_client(&ron::to_string(&message).unwrap()).is_err());
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
    assert_eq!(decode_server(&text_frame(&maximum)).unwrap(), maximum);
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
    // Full rooms are valid on the wire, though the server does not list them.
    room.players = 1;
    room.capacity = 1;
    room.map.clear();
    let message = ServerMessage::Rooms { rooms: vec![room] };
    assert_eq!(decode_server(&text_frame(&message)).unwrap(), message);
    assert!(
        decode_server(&format!(
            "{}{}",
            text_frame(&message),
            " ".repeat(MAX_MESSAGE_BYTES)
        ))
        .is_err()
    );
}

#[test]
fn lobby_validates_host_receiver_slot_count_names_and_unknown_fields() {
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
    assert_eq!(decode_server(&text_frame(&valid)).unwrap(), valid);
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
    for message in [
        "Rooms(rooms:[(code:\"AB12C3\",host:\"Host\",map:\"Map\",players:1,capacity:2,extra:0)])",
        "Lobby(code:\"AB12C3\",player:(0),public:true,slots:[Some((name:\"Host\",ready:false,extra:0))])",
        "Rooms(rooms:[],extra:0)",
    ] {
        assert!(decode_server(message).is_err(), "accepted {message}");
    }
    for message in [
        "Rooms(version:5,extra:0)",
        "Create(version:5,name:\"Host\",public:true,extra:0)",
        "Join(version:5,code:\"AB12C3\",name:\"Guest\",extra:0)",
        "Loaded(checksum:0,extra:0)",
        "Ready(ready:true,extra:0)",
        "Ready(checksum:0)",
        "Rooms(version:4)",
    ] {
        assert!(decode_client(message).is_err(), "accepted {message}");
    }
    assert!(
        decode_server(&format!(
            "{}{}",
            text_frame(&valid),
            " ".repeat(MAX_MESSAGE_BYTES)
        ))
        .is_err()
    );
}
