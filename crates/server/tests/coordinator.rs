#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use rts_base::{Command, PlayerId, UnitId};
use rts_net::{
    ClientMessage, MAX_COMMANDS_PER_TURN, MAX_UNITS_PER_COMMAND, PROTOCOL_VERSION, ServerMessage,
};
use rts_server::{Config, Coordinator, Delivery, MAX_FUTURE_TICKS};

const NOW: Duration = Duration::ZERO;

fn receive(relay: &mut Coordinator, client: u64, message: ClientMessage) -> Vec<Delivery> {
    relay.receive(client, message, NOW)
}

fn create(relay: &mut Coordinator, client: u64, players: u8) -> String {
    let messages = receive(
        relay,
        client,
        ClientMessage::Create {
            version: PROTOCOL_VERSION,
            checksum: 17,
            players,
        },
    );
    let ServerMessage::Lobby { code, player, .. } = &messages[0].message else {
        panic!("expected lobby")
    };
    assert_eq!(*player, PlayerId(0));
    assert_eq!(code.len(), 6);
    assert!(
        code.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    );
    code.clone()
}

fn join(relay: &mut Coordinator, client: u64, code: &str) -> Vec<Delivery> {
    receive(
        relay,
        client,
        ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: code.to_owned(),
            checksum: 17,
        },
    )
}

fn lobby() -> (Coordinator, String) {
    let mut relay = Coordinator::new(Config::default());
    for id in 1..=3 {
        assert!(relay.connect(id, NOW));
    }
    let code = create(&mut relay, 1, 2);
    (relay, code)
}

fn running() -> Coordinator {
    let (mut relay, code) = lobby();
    join(&mut relay, 2, &code);
    let messages = receive(&mut relay, 1, ClientMessage::Start);
    assert_eq!(messages.len(), 4);
    for message in &messages[0..2] {
        assert_eq!(
            message.message,
            ServerMessage::Started {
                players: 2,
                input_delay: 2
            }
        );
    }
    for message in &messages[2..4] {
        let ServerMessage::Turn(turn) = &message.message else {
            panic!("expected bootstrap turn")
        };
        turn.validate(1, 2).unwrap();
        assert!(turn.inputs.iter().all(|input| input.commands.is_empty()));
    }
    relay
}

fn assert_error(messages: &[Delivery], client: u64) {
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].client, client);
    assert!(matches!(messages[0].message, ServerMessage::Error { .. }));
}

fn assert_ended(messages: &[Delivery], expected: usize) {
    assert_eq!(messages.len(), expected);
    assert!(
        messages
            .iter()
            .all(|delivery| matches!(delivery.message, ServerMessage::Ended { .. }))
    );
}

#[test]
fn joins_retry_and_only_host_starts_a_full_room() {
    let (mut relay, code) = lobby();
    assert_error(&receive(&mut relay, 1, ClientMessage::Start), 1);
    assert_error(
        &receive(
            &mut relay,
            2,
            ClientMessage::Join {
                version: PROTOCOL_VERSION + 1,
                code: code.clone(),
                checksum: 17,
            },
        ),
        2,
    );
    assert_error(
        &receive(
            &mut relay,
            2,
            ClientMessage::Join {
                version: PROTOCOL_VERSION,
                code: code.clone(),
                checksum: 18,
            },
        ),
        2,
    );
    assert_error(&join(&mut relay, 2, "bad"), 2);
    assert_error(&join(&mut relay, 2, "999999"), 2);
    assert_eq!(join(&mut relay, 2, &code).len(), 2);
    assert_error(&join(&mut relay, 2, &code), 2);
    assert_error(&join(&mut relay, 3, &code), 3);
    assert_error(&receive(&mut relay, 2, ClientMessage::Start), 2);
    assert_eq!(receive(&mut relay, 1, ClientMessage::Start).len(), 4);
    assert_error(&join(&mut relay, 3, &code), 3);
    assert_error(&receive(&mut relay, 1, ClientMessage::Start), 1);
}

#[test]
fn guest_departure_reuses_slot_and_host_departure_ends_room() {
    for explicit_leave in [false, true] {
        let (mut relay, code) = lobby();
        join(&mut relay, 2, &code);
        let updates = if explicit_leave {
            receive(&mut relay, 2, ClientMessage::Leave)
        } else {
            relay.disconnect(2)
        };
        assert!(updates.iter().any(|delivery| delivery.client == 1
            && matches!(
                delivery.message,
                ServerMessage::Lobby {
                    players: 1,
                    capacity: 2,
                    ..
                }
            )));
        let updates = join(&mut relay, 3, &code);
        assert!(updates.iter().any(|delivery| delivery.client == 3
            && matches!(
                delivery.message,
                ServerMessage::Lobby {
                    player: PlayerId(1),
                    ..
                }
            )));
        assert_ended(&relay.disconnect(1), 2);
        // The surviving guest can create another room on its same connection.
        create(&mut relay, 3, 1);
    }
}

#[test]
fn authenticated_slots_sort_inputs_and_preserve_command_order() {
    let mut relay = running();
    let commands = vec![
        Command::Stop {
            units: vec![UnitId(8)],
        },
        Command::Stop {
            units: vec![UnitId(3)],
        },
    ];
    assert!(
        receive(
            &mut relay,
            2,
            ClientMessage::Input {
                tick: 2,
                commands: commands.clone()
            }
        )
        .is_empty()
    );
    let messages = receive(
        &mut relay,
        1,
        ClientMessage::Input {
            tick: 2,
            commands: Vec::new(),
        },
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].message, messages[1].message);
    let ServerMessage::Turn(turn) = &messages[0].message else {
        panic!("expected turn")
    };
    turn.validate(2, 2).unwrap();
    assert_eq!(turn.inputs[0].commands, []);
    assert_eq!(turn.inputs[1].commands, commands);
    assert_error(
        &receive(
            &mut relay,
            3,
            ClientMessage::Input {
                tick: 3,
                commands: Vec::new(),
            },
        ),
        3,
    );
}

#[test]
fn invalid_inputs_do_not_consume_ticks_and_future_buffers_are_bounded() {
    let mut relay = running();
    for tick in [0, 1, 3, u32::MAX] {
        assert_error(
            &receive(
                &mut relay,
                1,
                ClientMessage::Input {
                    tick,
                    commands: Vec::new(),
                },
            ),
            1,
        );
    }
    for commands in [
        vec![Command::Stop { units: Vec::new() }; MAX_COMMANDS_PER_TURN + 1],
        vec![Command::Stop {
            units: vec![UnitId(0); MAX_UNITS_PER_COMMAND + 1],
        }],
    ] {
        assert_error(
            &receive(&mut relay, 1, ClientMessage::Input { tick: 2, commands }),
            1,
        );
    }
    for tick in 2..=MAX_FUTURE_TICKS {
        assert!(
            receive(
                &mut relay,
                1,
                ClientMessage::Input {
                    tick,
                    commands: Vec::new()
                }
            )
            .is_empty()
        );
    }
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Input {
                tick: MAX_FUTURE_TICKS + 1,
                commands: Vec::new(),
            },
        ),
        1,
    );
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Input {
                tick: 2,
                commands: Vec::new(),
            },
        ),
        1,
    );
    // A gap rejection leaves the guest's expected tick unchanged.
    assert_error(
        &receive(
            &mut relay,
            2,
            ClientMessage::Input {
                tick: 3,
                commands: Vec::new(),
            },
        ),
        2,
    );
    assert_eq!(
        receive(
            &mut relay,
            2,
            ClientMessage::Input {
                tick: 2,
                commands: Vec::new()
            }
        )
        .len(),
        2
    );
}

#[test]
fn hashes_confirm_once_and_reject_gaps_duplicates_and_unemitted_turns() {
    let mut relay = running();
    for tick in [0, 2, u32::MAX] {
        assert_error(
            &receive(&mut relay, 1, ClientMessage::Hash { tick, hash: 99 }),
            1,
        );
    }
    assert!(receive(&mut relay, 2, ClientMessage::Hash { tick: 1, hash: 99 }).is_empty());
    assert_error(
        &receive(&mut relay, 2, ClientMessage::Hash { tick: 1, hash: 99 }),
        2,
    );
    let messages = receive(&mut relay, 1, ClientMessage::Hash { tick: 1, hash: 99 });
    assert_eq!(messages.len(), 2);
    assert!(
        messages
            .iter()
            .all(|delivery| delivery.message == ServerMessage::HashConfirmed { tick: 1, hash: 99 })
    );
    assert_error(
        &receive(&mut relay, 1, ClientMessage::Hash { tick: 1, hash: 99 }),
        1,
    );
}

#[test]
fn hash_lead_is_bounded_even_if_other_peer_never_reports() {
    let mut relay = running();
    for tick in 1..=MAX_FUTURE_TICKS {
        if tick > 1 {
            receive(
                &mut relay,
                1,
                ClientMessage::Input {
                    tick,
                    commands: Vec::new(),
                },
            );
            receive(
                &mut relay,
                2,
                ClientMessage::Input {
                    tick,
                    commands: Vec::new(),
                },
            );
        }
        assert!(receive(&mut relay, 1, ClientMessage::Hash { tick, hash: 99 }).is_empty());
    }
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Hash {
                tick: MAX_FUTURE_TICKS + 1,
                hash: 99,
            },
        ),
        1,
    );
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Input {
                tick: MAX_FUTURE_TICKS + 1,
                commands: Vec::new(),
            },
        ),
        1,
    );
    // Completing hashes releases the buffer and allows the rejected tick to retry.
    for tick in 1..=MAX_FUTURE_TICKS {
        assert_eq!(
            receive(&mut relay, 2, ClientMessage::Hash { tick, hash: 99 }).len(),
            2
        );
    }
    assert!(
        receive(
            &mut relay,
            1,
            ClientMessage::Input {
                tick: MAX_FUTURE_TICKS + 1,
                commands: Vec::new()
            }
        )
        .is_empty()
    );
}

#[test]
fn mismatch_disconnect_and_leave_end_every_running_room() {
    let mut relay = running();
    receive(&mut relay, 1, ClientMessage::Hash { tick: 1, hash: 42 });
    let messages = receive(&mut relay, 2, ClientMessage::Hash { tick: 1, hash: 43 });
    assert_ended(&messages, 2);
    assert!(messages.iter().all(|delivery| matches!(&delivery.message,
        ServerMessage::Ended { reason } if reason.contains("mismatch"))));
    create(&mut relay, 1, 1);
    let mut relay = running();
    assert_ended(&relay.disconnect(2), 2);
    let mut relay = running();
    assert_ended(&receive(&mut relay, 2, ClientMessage::Leave), 2);
}

#[test]
fn timeout_distinguishes_waiting_lobbies_silent_sockets_and_stalled_games() {
    let (mut relay, code) = lobby();
    assert!(relay.expire(Duration::from_secs(16)).is_empty());
    assert!(relay.is_connected(1));
    assert!(!relay.is_connected(2)); // Unjoined sockets have the shorter timeout.
    assert!(relay.connect(2, Duration::from_secs(16)));
    join(&mut relay, 2, &code);
    assert!(relay.expire(Duration::from_secs(1799)).is_empty());
    assert_ended(&relay.expire(Duration::from_mins(30)), 2);
    assert!(!relay.is_connected(1));

    let mut relay = running();
    // Pings and invalid requests cannot extend a match with no valid progress.
    for id in 1..=2 {
        relay.touch(id, Duration::from_secs(14));
    }
    assert_ended(&relay.expire(Duration::from_secs(15)), 2);
    assert!(relay.is_connected(1));

    let mut relay = running();
    relay.touch(1, Duration::from_secs(14));
    assert_ended(&relay.expire(Duration::from_secs(15)), 2);
    assert!(!relay.is_connected(2));
}

#[test]
fn starting_after_a_long_lobby_wait_resets_every_slots_heartbeat_window() {
    let (mut relay, code) = lobby();
    join(&mut relay, 2, &code);
    assert!(relay.expire(Duration::from_secs(100)).is_empty());
    assert_eq!(
        relay
            .receive(1, ClientMessage::Start, Duration::from_secs(100))
            .len(),
        4
    );
    assert!(relay.expire(Duration::from_secs(114)).is_empty());
    assert!(relay.is_connected(2));
    assert_ended(&relay.expire(Duration::from_secs(115)), 2);
}

#[test]
fn input_and_hash_progress_have_independent_stall_deadlines() {
    for only_hashes in [false, true] {
        let mut relay = running();
        for client in 1..=2 {
            let message = if only_hashes {
                ClientMessage::Hash { tick: 1, hash: 17 }
            } else {
                ClientMessage::Input {
                    tick: 2,
                    commands: Vec::new(),
                }
            };
            relay.receive(client, message, Duration::from_secs(14));
        }
        // Socket activity and progress on only one stream cannot hide a stall
        // on the other stream.
        assert_ended(&relay.expire(Duration::from_secs(15)), 2);
        assert!(relay.is_connected(1));
        assert!(relay.is_connected(2));
    }
}

#[test]
fn room_and_connection_limits_are_enforced_and_room_codes_are_unique() {
    let mut relay = Coordinator::new(Config {
        max_connections: 2,
        max_rooms: 1,
        ..Config::default()
    });
    assert!(relay.connect(1, NOW));
    assert!(!relay.connect(1, NOW));
    assert!(relay.connect(2, NOW));
    assert!(!relay.connect(3, NOW));
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Create {
                version: 1,
                checksum: 17,
                players: 0,
            },
        ),
        1,
    );
    assert_error(
        &receive(
            &mut relay,
            1,
            ClientMessage::Create {
                version: 1,
                checksum: 17,
                players: 9,
            },
        ),
        1,
    );
    let code = create(&mut relay, 1, 1);
    assert_error(
        &receive(
            &mut relay,
            2,
            ClientMessage::Create {
                version: 1,
                checksum: 17,
                players: 1,
            },
        ),
        2,
    );
    relay.disconnect(1);
    let next_code = create(&mut relay, 2, 1);
    assert_ne!(code, next_code);
}
