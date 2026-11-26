//! Compact binary wire framing for low-bandwidth multiplayer.
//!
//! Every message starts with a single header byte: bits `0..7` carry the message tag and bit `7`
//! (`0x80`) marks whether the payload is raw-deflate compressed. Small messages (such as quiet
//! view frames or single commands) travel uncompressed in a few bytes; larger payloads (such as
//! lobby setups, opening fog discoveries, or large battles) compress automatically whenever
//! deflate makes them smaller.

use miniz_oxide::{
    deflate::compress_to_vec,
    inflate::{
        TINFLStatus,
        core::{DecompressorOxide, decompress, inflate_flags},
    },
};
use rts_base::{
    Command, DepositId, IVec2, MatchOutcome, OwnUnitState, PlayerId, ResourceKind, Resources, Shot,
    TerrainId, Training, TrainingKind, Unit, UnitId, UnitKind, ViewDeposit, ViewFrame, ViewIntro,
    ViewUnit, Volley,
};

use crate::{
    ClientMessage, LobbySetup, LobbySlot, MAX_COMMANDS_PER_TURN, MAX_LISTED_ROOMS,
    MAX_MAP_NAME_CHARS, MAX_MESSAGE_BYTES, MAX_SETUP_BYTES, MAX_UNITS_PER_COMMAND,
    MAX_VIEW_MESSAGE_BYTES, NetError, PROTOCOL_VERSION, RoomInfo, ServerMessage, check_size,
    validate_map_name, validate_players, validate_version,
};

const COMPRESSED_BIT: u8 = 0x80;
const TAG_MASK: u8 = 0x7f;
/// Only attempt deflate when the raw payload is at least this many bytes.
const COMPRESS_THRESHOLD: usize = 64;

/// Maximum wire size of any server message, including minified rules setup.
pub const MAX_SERVER_FRAME_BYTES: usize = MAX_SETUP_BYTES + 65_536;

// Client message tags.
const C_ROOMS: u8 = 0;
const C_CREATE: u8 = 1;
const C_JOIN: u8 = 2;
const C_LOADED: u8 = 3;
const C_READY: u8 = 4;
const C_START: u8 = 5;
const C_INPUT: u8 = 6;
const C_LEAVE: u8 = 7;

// Server message tags.
const S_SETUP: u8 = 0;
const S_ROOMS: u8 = 1;
const S_LOBBY: u8 = 2;
const S_STARTED: u8 = 3;
const S_VIEW: u8 = 4;
const S_ENDED: u8 = 5;
const S_ERROR: u8 = 6;

// Command tags.
const CMD_MOVE: u8 = 0;
const CMD_STOP: u8 = 1;
const CMD_ATTACK: u8 = 2;
const CMD_GATHER: u8 = 3;
const CMD_BUILD_BARRACKS: u8 = 4;
const CMD_TRAIN_WORKER: u8 = 5;
const CMD_TRAIN_RIFLE_SQUAD: u8 = 6;
const CMD_TRAIN_SCOUT: u8 = 7;

// ViewFrame presence bits.
const VF_EXPLORED: u8 = 1 << 0;
const VF_UNITS: u8 = 1 << 1;
const VF_REMOVED: u8 = 1 << 2;
const VF_DEPOSITS: u8 = 1 << 3;
const VF_RESOURCES: u8 = 1 << 4;
const VF_HEADQUARTERS: u8 = 1 << 5;
const VF_OUTCOME: u8 = 1 << 6;
const VF_VOLLEYS: u8 = 1 << 7;

// ViewUnit flags.
const VU_OWN: u8 = 1 << 0;
const VU_COOLDOWN: u8 = 1 << 1;
const VU_KILLS: u8 = 1 << 2;
const VU_UNIFORM_SOLDIERS: u8 = 1 << 3;
const VU_COMPACT_KIND_OWNER: u8 = 1 << 4;

// OwnUnitState presence bits.
const OWN_MOVE: u8 = 1 << 0;
const OWN_ATTACK: u8 = 1 << 1;
const OWN_GATHER: u8 = 1 << 2;
const OWN_BUILD: u8 = 1 << 3;
const OWN_CONSTRUCTION: u8 = 1 << 4;
const OWN_CARGO: u8 = 1 << 5;
const OWN_PRODUCTION: u8 = 1 << 6;

/// Encodes a validated client message into compact binary wire bytes.
pub fn encode_client(message: &ClientMessage) -> Result<Vec<u8>, NetError> {
    let mut payload = Writer::new(MAX_MESSAGE_BYTES);
    let tag = match message {
        ClientMessage::Rooms { version } => {
            payload.write_u32(*version)?;
            C_ROOMS
        }
        ClientMessage::Create {
            version,
            name,
            public,
        } => {
            payload.write_u32(*version)?;
            payload.write_bool(*public)?;
            payload.write_str(name)?;
            C_CREATE
        }
        ClientMessage::Join {
            version,
            code,
            name,
        } => {
            payload.write_u32(*version)?;
            payload.write_str(code)?;
            payload.write_str(name)?;
            C_JOIN
        }
        ClientMessage::Loaded { checksum } => {
            payload.write_u64(*checksum)?;
            C_LOADED
        }
        ClientMessage::Ready { ready } => {
            payload.write_bool(*ready)?;
            C_READY
        }
        ClientMessage::Start => C_START,
        ClientMessage::Input { tick, commands } => {
            payload.write_u32(*tick)?;
            payload.write_len(commands.len())?;
            for command in commands {
                write_command(&mut payload, command)?;
            }
            C_INPUT
        }
        ClientMessage::Leave => C_LEAVE,
    };
    finish_frame(tag, &payload.buf, MAX_MESSAGE_BYTES)
}

/// Decodes a client message from compact binary wire bytes.
pub fn decode_client(bytes: &[u8]) -> Result<ClientMessage, NetError> {
    check_size(bytes.len(), MAX_MESSAGE_BYTES)?;
    let (tag, decoded) = open_frame(bytes, MAX_MESSAGE_BYTES)?;
    let mut r = Reader::new(&decoded);
    let message = match tag {
        C_ROOMS => ClientMessage::Rooms {
            version: r.read_u32()?,
        },
        C_CREATE => {
            let version = r.read_u32()?;
            let public = r.read_bool()?;
            let name = r.read_str(MAX_MESSAGE_BYTES)?;
            ClientMessage::Create {
                version,
                name,
                public,
            }
        }
        C_JOIN => {
            let version = r.read_u32()?;
            let code = r.read_str(16)?;
            let name = r.read_str(MAX_MESSAGE_BYTES)?;
            ClientMessage::Join {
                version,
                code,
                name,
            }
        }
        C_LOADED => ClientMessage::Loaded {
            checksum: r.read_u64()?,
        },
        C_READY => ClientMessage::Ready {
            ready: r.read_bool()?,
        },
        C_START => ClientMessage::Start,
        C_INPUT => {
            let tick = r.read_u32()?;
            let count = r.read_len(MAX_COMMANDS_PER_TURN)?;
            let mut commands = Vec::with_capacity(count);
            for _ in 0..count {
                commands.push(read_command(&mut r)?);
            }
            ClientMessage::Input { tick, commands }
        }
        C_LEAVE => ClientMessage::Leave,
        _ => return Err(NetError::invalid("unknown client message tag")),
    };
    r.finish()?;
    Ok(message)
}

/// Encodes a validated server message into compact binary wire bytes.
pub fn encode_server(message: &ServerMessage) -> Result<Vec<u8>, NetError> {
    let limit = server_limit(message);
    let mut payload = Writer::new(limit);
    let tag = match message {
        ServerMessage::Setup(setup) => {
            let compact = setup.minified();
            payload.write_u32(PROTOCOL_VERSION)?;
            payload.write_u8(compact.players)?;
            payload.write_str(&compact.map)?;
            payload.write_str(&compact.rules)?;
            S_SETUP
        }
        ServerMessage::Rooms { rooms } => {
            payload.write_len(rooms.len())?;
            for room in rooms {
                payload.write_str(&room.code)?;
                payload.write_str(&room.host)?;
                payload.write_str(&room.map)?;
                payload.write_u8(room.players)?;
                payload.write_u8(room.capacity)?;
            }
            S_ROOMS
        }
        ServerMessage::Lobby {
            code,
            player,
            public,
            slots,
        } => {
            payload.write_str(code)?;
            payload.write_u8(player.0)?;
            payload.write_bool(*public)?;
            payload.write_len(slots.len())?;
            for slot in slots {
                match slot {
                    None => payload.write_u8(0)?,
                    Some(slot) => {
                        payload.write_u8(if slot.ready { 2 } else { 1 })?;
                        payload.write_str(&slot.name)?;
                    }
                }
            }
            S_LOBBY
        }
        ServerMessage::Started {
            players,
            input_delay,
            view,
        } => {
            payload.write_u8(*players)?;
            payload.write_u32(*input_delay)?;
            write_view_intro(&mut payload, *view)?;
            S_STARTED
        }
        ServerMessage::View { frame } => {
            write_view_frame(&mut payload, frame)?;
            S_VIEW
        }
        ServerMessage::Ended { reason } => {
            payload.write_str(reason)?;
            S_ENDED
        }
        ServerMessage::Error { message } => {
            payload.write_str(message)?;
            S_ERROR
        }
    };
    finish_frame(tag, &payload.buf, limit)
}

/// Decodes a server message from compact binary wire bytes.
pub fn decode_server(bytes: &[u8]) -> Result<ServerMessage, NetError> {
    let Some(&first) = bytes.first() else {
        return Err(NetError::invalid("empty server message"));
    };
    let tag = first & TAG_MASK;
    let limit = server_tag_limit(tag)?;
    check_size(bytes.len(), limit)?;
    let (_, decoded) = open_frame(bytes, limit)?;
    let mut r = Reader::new(&decoded);
    let message = match tag {
        S_SETUP => {
            validate_version(r.read_u32()?)?;
            let players = r.read_u8()?;
            validate_players(players)?;
            let map = r.read_str(MAX_MAP_NAME_CHARS * 4)?;
            validate_map_name(&map)?;
            let rules = r.read_str(MAX_SETUP_BYTES)?;
            ServerMessage::Setup(LobbySetup {
                rules,
                map,
                players,
            })
        }
        S_ROOMS => {
            let count = r.read_len(MAX_LISTED_ROOMS)?;
            let mut rooms = Vec::with_capacity(count);
            for _ in 0..count {
                rooms.push(RoomInfo {
                    code: r.read_str(16)?,
                    host: r.read_str(MAX_MESSAGE_BYTES)?,
                    map: r.read_str(MAX_MAP_NAME_CHARS * 4)?,
                    players: r.read_u8()?,
                    capacity: r.read_u8()?,
                });
            }
            ServerMessage::Rooms { rooms }
        }
        S_LOBBY => {
            let code = r.read_str(16)?;
            let player = PlayerId(r.read_u8()?);
            let public = r.read_bool()?;
            let count = r.read_len(rts_base::MAX_PLAYERS)?;
            let mut slots = Vec::with_capacity(count);
            for _ in 0..count {
                let state = r.read_u8()?;
                match state {
                    0 => slots.push(None),
                    1 | 2 => slots.push(Some(LobbySlot {
                        name: r.read_str(MAX_MESSAGE_BYTES)?,
                        ready: state == 2,
                    })),
                    _ => return Err(NetError::invalid("invalid lobby slot state")),
                }
            }
            ServerMessage::Lobby {
                code,
                player,
                public,
                slots,
            }
        }
        S_STARTED => {
            let players = r.read_u8()?;
            let input_delay = r.read_u32()?;
            let view = read_view_intro(&mut r)?;
            ServerMessage::Started {
                players,
                input_delay,
                view,
            }
        }
        S_VIEW => ServerMessage::View {
            frame: read_view_frame(&mut r)?,
        },
        S_ENDED => ServerMessage::Ended {
            reason: r.read_str(MAX_MESSAGE_BYTES)?,
        },
        S_ERROR => ServerMessage::Error {
            message: r.read_str(MAX_MESSAGE_BYTES)?,
        },
        _ => return Err(NetError::invalid("unknown server message tag")),
    };
    r.finish()?;
    Ok(message)
}

const fn server_limit(message: &ServerMessage) -> usize {
    match message {
        ServerMessage::Setup(_) => MAX_SERVER_FRAME_BYTES,
        ServerMessage::View { .. } => MAX_VIEW_MESSAGE_BYTES,
        _ => MAX_MESSAGE_BYTES,
    }
}

fn server_tag_limit(tag: u8) -> Result<usize, NetError> {
    match tag {
        S_SETUP => Ok(MAX_SERVER_FRAME_BYTES),
        S_VIEW => Ok(MAX_VIEW_MESSAGE_BYTES),
        S_ROOMS | S_LOBBY | S_STARTED | S_ENDED | S_ERROR => Ok(MAX_MESSAGE_BYTES),
        _ => Err(NetError::invalid("unknown server message tag")),
    }
}

/// Wraps a payload with its tag byte and optional raw-deflate compression when smaller.
fn finish_frame(tag: u8, payload: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    debug_assert!(tag <= TAG_MASK);
    if payload.len() >= COMPRESS_THRESHOLD {
        let compressed = compress_to_vec(payload, 6);
        let mut header = Writer::new(16);
        header.write_u8(tag | COMPRESSED_BIT)?;
        header.write_len(payload.len())?;
        if header.buf.len() + compressed.len() < 1 + payload.len() {
            let total = header.buf.len() + compressed.len();
            check_size(total, limit)?;
            let mut out = Vec::with_capacity(total);
            out.extend_from_slice(&header.buf);
            out.extend_from_slice(&compressed);
            return Ok(out);
        }
    }
    let total = 1 + payload.len();
    check_size(total, limit)?;
    let mut out = Vec::with_capacity(total);
    out.push(tag);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Unwraps the header byte and decompresses if the compressed flag is set.
fn open_frame(bytes: &[u8], limit: usize) -> Result<(u8, Vec<u8>), NetError> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Err(NetError::invalid("empty wire message"));
    };
    let tag = first & TAG_MASK;
    if first & COMPRESSED_BIT == 0 {
        return Ok((tag, rest.to_vec()));
    }
    let mut header = Reader::new(rest);
    let expected_len = header.read_len(limit)?;
    let compressed = header.remaining();
    let inflated = inflate_bounded(expected_len, compressed, limit)?;
    Ok((tag, inflated))
}

fn inflate_bounded(len: usize, compressed: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    check_size(len, limit)?;
    check_size(compressed.len(), limit)?;
    let mut output = vec![0; len];
    let (status, consumed, written) = decompress(
        &mut Box::<DecompressorOxide>::default(),
        compressed,
        &mut output,
        0,
        inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    if status != TINFLStatus::Done || consumed != compressed.len() || written != len {
        return Err(NetError::invalid("invalid compressed wire payload"));
    }
    Ok(output)
}

fn write_command(w: &mut Writer, command: &Command) -> Result<(), NetError> {
    match command {
        Command::Move { units, target } => {
            w.write_u8(CMD_MOVE)?;
            write_unit_ids(w, units)?;
            w.write_ivec2(*target)?;
        }
        Command::Stop { units } => {
            w.write_u8(CMD_STOP)?;
            write_unit_ids(w, units)?;
        }
        Command::Attack { units, target } => {
            w.write_u8(CMD_ATTACK)?;
            write_unit_ids(w, units)?;
            w.write_u32(target.0)?;
        }
        Command::Gather { units, deposit } => {
            w.write_u8(CMD_GATHER)?;
            write_unit_ids(w, units)?;
            w.write_u32(deposit.0)?;
        }
        Command::BuildBarracks { units, target } => {
            w.write_u8(CMD_BUILD_BARRACKS)?;
            write_unit_ids(w, units)?;
            w.write_ivec2(*target)?;
        }
        Command::TrainWorker { units } => {
            w.write_u8(CMD_TRAIN_WORKER)?;
            write_unit_ids(w, units)?;
        }
        Command::TrainRifleSquad { units } => {
            w.write_u8(CMD_TRAIN_RIFLE_SQUAD)?;
            write_unit_ids(w, units)?;
        }
        Command::TrainScout { units } => {
            w.write_u8(CMD_TRAIN_SCOUT)?;
            write_unit_ids(w, units)?;
        }
    }
    Ok(())
}

fn read_command(r: &mut Reader<'_>) -> Result<Command, NetError> {
    let tag = r.read_u8()?;
    let units = read_unit_ids(r, MAX_UNITS_PER_COMMAND)?;
    match tag {
        CMD_MOVE => Ok(Command::Move {
            units,
            target: r.read_ivec2()?,
        }),
        CMD_STOP => Ok(Command::Stop { units }),
        CMD_ATTACK => Ok(Command::Attack {
            units,
            target: UnitId(r.read_u32()?),
        }),
        CMD_GATHER => Ok(Command::Gather {
            units,
            deposit: DepositId(r.read_u32()?),
        }),
        CMD_BUILD_BARRACKS => Ok(Command::BuildBarracks {
            units,
            target: r.read_ivec2()?,
        }),
        CMD_TRAIN_WORKER => Ok(Command::TrainWorker { units }),
        CMD_TRAIN_RIFLE_SQUAD => Ok(Command::TrainRifleSquad { units }),
        CMD_TRAIN_SCOUT => Ok(Command::TrainScout { units }),
        _ => Err(NetError::invalid("unknown command tag")),
    }
}

fn write_unit_ids(w: &mut Writer, ids: &[UnitId]) -> Result<(), NetError> {
    w.write_len(ids.len())?;
    let mut prev: i64 = 0;
    for &id in ids {
        let value = i64::from(id.0);
        w.write_i64(value - prev)?;
        prev = value + 1;
    }
    Ok(())
}

fn read_unit_ids(r: &mut Reader<'_>, max: usize) -> Result<Vec<UnitId>, NetError> {
    let count = r.read_len(max)?;
    let mut ids = Vec::with_capacity(count.min(r.remaining().len()));
    let mut prev: i64 = 0;
    for _ in 0..count {
        let delta = r.read_i64()?;
        let value = prev
            .checked_add(delta)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| NetError::invalid("unit id out of range"))?;
        ids.push(UnitId(value));
        prev = i64::from(value) + 1;
    }
    Ok(ids)
}

fn write_view_intro(w: &mut Writer, intro: ViewIntro) -> Result<(), NetError> {
    w.write_u8(intro.player.0)?;
    w.write_u8(intro.players)?;
    w.write_u8(u8::from(intro.skirmish) | (u8::from(intro.fog_disabled) << 1))?;
    w.write_ivec2(intro.size)?;
    w.write_ivec2(intro.start)?;
    Ok(())
}

fn read_view_intro(r: &mut Reader<'_>) -> Result<ViewIntro, NetError> {
    let player = PlayerId(r.read_u8()?);
    let players = r.read_u8()?;
    let flags = r.read_u8()?;
    if flags & !0b11 != 0 {
        return Err(NetError::invalid("unknown view intro flags"));
    }
    let size = r.read_ivec2()?;
    let start = r.read_ivec2()?;
    Ok(ViewIntro {
        player,
        players,
        size,
        start,
        skirmish: flags & 1 != 0,
        fog_disabled: flags & 2 != 0,
    })
}

fn write_view_frame(w: &mut Writer, frame: &ViewFrame) -> Result<(), NetError> {
    w.write_u32(frame.tick)?;
    let mut flags = 0_u8;
    if !frame.explored.is_empty() {
        flags |= VF_EXPLORED;
    }
    if !frame.units.is_empty() {
        flags |= VF_UNITS;
    }
    if !frame.removed.is_empty() {
        flags |= VF_REMOVED;
    }
    if !frame.deposits.is_empty() {
        flags |= VF_DEPOSITS;
    }
    if frame.resources.is_some() {
        flags |= VF_RESOURCES;
    }
    if frame.headquarters.is_some() {
        flags |= VF_HEADQUARTERS;
    }
    if frame.outcome.is_some() {
        flags |= VF_OUTCOME;
    }
    if !frame.volleys.is_empty() {
        flags |= VF_VOLLEYS;
    }
    w.write_u8(flags)?;

    if !frame.explored.is_empty() {
        w.write_len(frame.explored.len())?;
        let mut prev: i64 = 0;
        for &(index, terrain) in &frame.explored {
            let value = i64::from(index);
            w.write_i64(value - prev)?;
            w.write_u8(terrain.0)?;
            prev = value + 1;
        }
    }
    if !frame.units.is_empty() {
        w.write_len(frame.units.len())?;
        let mut prev_id: i64 = 0;
        let mut prev_pos = IVec2::ZERO;
        for view_unit in &frame.units {
            write_view_unit(w, view_unit, &mut prev_id, &mut prev_pos)?;
        }
    }
    if !frame.removed.is_empty() {
        write_unit_ids(w, &frame.removed)?;
    }
    if !frame.deposits.is_empty() {
        w.write_len(frame.deposits.len())?;
        let mut prev_id: i64 = 0;
        let mut prev_pos = IVec2::ZERO;
        for deposit in &frame.deposits {
            let value = i64::from(deposit.id.0);
            w.write_i64(value - prev_id)?;
            prev_id = value + 1;
            write_delta_pos(w, deposit.pos, prev_pos)?;
            prev_pos = deposit.pos;
            w.write_u8(resource_kind_byte(deposit.kind))?;
            w.write_u32(deposit.remaining)?;
        }
    }
    if let Some(resources) = frame.resources {
        w.write_u32(resources.wood)?;
        w.write_u32(resources.gold)?;
    }
    if let Some(hq) = frame.headquarters {
        w.write_u32(hq.0)?;
    }
    if let Some(outcome) = frame.outcome {
        match outcome {
            MatchOutcome::Draw => w.write_u8(0)?,
            MatchOutcome::Winner(player) => {
                w.write_u8(1)?;
                w.write_u8(player.0)?;
            }
        }
    }
    if !frame.volleys.is_empty() {
        w.write_len(frame.volleys.len())?;
        for volley in &frame.volleys {
            w.write_u32(volley.shooter.0)?;
            w.write_u32(u32::from(volley.shooter_kind.0))?;
            w.write_u32(volley.target.0)?;
            w.write_u32(u32::from(volley.target_kind.0))?;
            w.write_ivec2(volley.from)?;
            write_delta_pos(w, volley.to, volley.from)?;
            w.write_len(volley.shots.len())?;
            for shot in &volley.shots {
                w.write_u8(shot.soldier)?;
                w.write_u8(shot.hit)?;
            }
        }
    }
    Ok(())
}

fn read_view_frame(r: &mut Reader<'_>) -> Result<ViewFrame, NetError> {
    let tick = r.read_u32()?;
    let flags = r.read_u8()?;

    let explored = if flags & VF_EXPLORED != 0 {
        read_explored(r)?
    } else {
        Vec::new()
    };

    let units = if flags & VF_UNITS != 0 {
        let count = r.read_len(MAX_VIEW_MESSAGE_BYTES)?;
        let mut list = Vec::with_capacity(count.min(r.remaining().len() / 4));
        let mut prev_id: i64 = 0;
        let mut prev_pos = IVec2::ZERO;
        for _ in 0..count {
            list.push(read_view_unit(r, &mut prev_id, &mut prev_pos)?);
        }
        list
    } else {
        Vec::new()
    };

    let removed = if flags & VF_REMOVED != 0 {
        read_unit_ids(r, MAX_VIEW_MESSAGE_BYTES)?
    } else {
        Vec::new()
    };

    let deposits = if flags & VF_DEPOSITS != 0 {
        read_deposits(r)?
    } else {
        Vec::new()
    };

    let resources = if flags & VF_RESOURCES != 0 {
        Some(Resources {
            wood: r.read_u32()?,
            gold: r.read_u32()?,
        })
    } else {
        None
    };

    let headquarters = if flags & VF_HEADQUARTERS != 0 {
        Some(UnitId(r.read_u32()?))
    } else {
        None
    };

    let outcome = if flags & VF_OUTCOME != 0 {
        match r.read_u8()? {
            0 => Some(MatchOutcome::Draw),
            1 => Some(MatchOutcome::Winner(PlayerId(r.read_u8()?))),
            _ => return Err(NetError::invalid("invalid match outcome tag")),
        }
    } else {
        None
    };

    let volleys = if flags & VF_VOLLEYS != 0 {
        read_volleys(r)?
    } else {
        Vec::new()
    };

    Ok(ViewFrame {
        tick,
        explored,
        units,
        removed,
        deposits,
        resources,
        headquarters,
        outcome,
        volleys,
    })
}

fn read_explored(r: &mut Reader<'_>) -> Result<Vec<(u32, TerrainId)>, NetError> {
    let count = r.read_len(MAX_VIEW_MESSAGE_BYTES)?;
    let mut tiles = Vec::with_capacity(count.min(r.remaining().len() / 2));
    let mut prev: i64 = 0;
    for _ in 0..count {
        let delta = r.read_i64()?;
        let index = prev
            .checked_add(delta)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| NetError::invalid("explored tile index out of range"))?;
        let terrain = TerrainId(r.read_u8()?);
        tiles.push((index, terrain));
        prev = i64::from(index) + 1;
    }
    Ok(tiles)
}

fn read_deposits(r: &mut Reader<'_>) -> Result<Vec<ViewDeposit>, NetError> {
    let count = r.read_len(MAX_VIEW_MESSAGE_BYTES)?;
    let mut list = Vec::with_capacity(count.min(r.remaining().len() / 4));
    let mut prev_id: i64 = 0;
    let mut prev_pos = IVec2::ZERO;
    for _ in 0..count {
        let delta = r.read_i64()?;
        let id = prev_id
            .checked_add(delta)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| NetError::invalid("deposit id out of range"))?;
        prev_id = i64::from(id) + 1;
        let pos = read_delta_pos(r, prev_pos)?;
        prev_pos = pos;
        let kind = read_resource_kind(r.read_u8()?)?;
        let remaining = r.read_u32()?;
        list.push(ViewDeposit {
            id: DepositId(id),
            pos,
            kind,
            remaining,
        });
    }
    Ok(list)
}

fn read_volleys(r: &mut Reader<'_>) -> Result<Vec<Volley>, NetError> {
    let count = r.read_len(MAX_VIEW_MESSAGE_BYTES)?;
    let mut list = Vec::with_capacity(count.min(r.remaining().len() / 6));
    for _ in 0..count {
        let shooter = UnitId(r.read_u32()?);
        let shooter_kind = UnitKind(
            u16::try_from(r.read_u32()?)
                .map_err(|_| NetError::invalid("unit kind out of range"))?,
        );
        let target = UnitId(r.read_u32()?);
        let target_kind = UnitKind(
            u16::try_from(r.read_u32()?)
                .map_err(|_| NetError::invalid("unit kind out of range"))?,
        );
        let from = r.read_ivec2()?;
        let to = read_delta_pos(r, from)?;
        let shot_count = r.read_len(256)?;
        let mut shots = Vec::with_capacity(shot_count.min(r.remaining().len() / 2));
        for _ in 0..shot_count {
            shots.push(Shot {
                soldier: r.read_u8()?,
                hit: r.read_u8()?,
            });
        }
        list.push(Volley {
            shooter,
            shooter_kind,
            target,
            target_kind,
            from,
            to,
            shots,
        });
    }
    Ok(list)
}

fn write_view_unit(
    w: &mut Writer,
    view: &ViewUnit,
    prev_id: &mut i64,
    prev_pos: &mut IVec2,
) -> Result<(), NetError> {
    let id = i64::from(view.id.0);
    w.write_i64(id - *prev_id)?;
    *prev_id = id + 1;

    let unit = &view.unit;
    let uniform_soldiers = unit
        .soldiers
        .first()
        .is_none_or(|first| unit.soldiers.iter().all(|health| health == first));
    let compact_kind_owner = unit.kind.0 < 16 && unit.owner.0 < 16;

    let mut flags = 0_u8;
    if view.own.is_some() {
        flags |= VU_OWN;
    }
    if unit.cooldown != 0 {
        flags |= VU_COOLDOWN;
    }
    if unit.kills != 0 {
        flags |= VU_KILLS;
    }
    if uniform_soldiers {
        flags |= VU_UNIFORM_SOLDIERS;
    }
    if compact_kind_owner {
        flags |= VU_COMPACT_KIND_OWNER;
    }
    w.write_u8(flags)?;

    if compact_kind_owner {
        let packed = (unit.owner.0 << 4) | u8::try_from(unit.kind.0).expect("checked < 16");
        w.write_u8(packed)?;
    } else {
        w.write_u32(u32::from(unit.kind.0))?;
        w.write_u8(unit.owner.0)?;
    }

    write_delta_pos(w, unit.pos, *prev_pos)?;
    *prev_pos = unit.pos;

    w.write_len(unit.soldiers.len())?;
    if uniform_soldiers {
        if let Some(&health) = unit.soldiers.first() {
            w.write_i32(health)?;
        }
    } else {
        for &health in &unit.soldiers {
            w.write_i32(health)?;
        }
    }
    if unit.cooldown != 0 {
        w.write_u32(unit.cooldown)?;
    }
    if unit.kills != 0 {
        w.write_u32(unit.kills)?;
    }
    if let Some(own) = &view.own {
        write_own_unit_state(w, own, unit.pos)?;
    }
    Ok(())
}

fn read_view_unit(
    r: &mut Reader<'_>,
    prev_id: &mut i64,
    prev_pos: &mut IVec2,
) -> Result<ViewUnit, NetError> {
    let delta = r.read_i64()?;
    let id = prev_id
        .checked_add(delta)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| NetError::invalid("unit id out of range"))?;
    *prev_id = i64::from(id) + 1;

    let flags = r.read_u8()?;
    let (kind, owner) = if flags & VU_COMPACT_KIND_OWNER != 0 {
        let packed = r.read_u8()?;
        (UnitKind(u16::from(packed & 0x0f)), PlayerId(packed >> 4))
    } else {
        let kind = u16::try_from(r.read_u32()?)
            .map_err(|_| NetError::invalid("unit kind out of range"))?;
        let owner = r.read_u8()?;
        (UnitKind(kind), PlayerId(owner))
    };

    let pos = read_delta_pos(r, *prev_pos)?;
    *prev_pos = pos;

    let soldier_count = r.read_len(256)?;
    let soldiers = if flags & VU_UNIFORM_SOLDIERS != 0 {
        if soldier_count == 0 {
            Vec::new()
        } else {
            vec![r.read_i32()?; soldier_count]
        }
    } else {
        let mut list = Vec::with_capacity(soldier_count.min(r.remaining().len()));
        for _ in 0..soldier_count {
            list.push(r.read_i32()?);
        }
        list
    };
    let cooldown = if flags & VU_COOLDOWN != 0 {
        r.read_u32()?
    } else {
        0
    };
    let kills = if flags & VU_KILLS != 0 {
        r.read_u32()?
    } else {
        0
    };
    let own = if flags & VU_OWN != 0 {
        Some(read_own_unit_state(r, pos)?)
    } else {
        None
    };

    Ok(ViewUnit {
        id: UnitId(id),
        unit: Unit {
            kind,
            owner,
            pos,
            soldiers,
            cooldown,
            kills,
        },
        own,
    })
}

fn write_own_unit_state(w: &mut Writer, own: &OwnUnitState, origin: IVec2) -> Result<(), NetError> {
    let mut flags = 0_u8;
    if own.move_target.is_some() {
        flags |= OWN_MOVE;
    }
    if own.attack_target.is_some() {
        flags |= OWN_ATTACK;
    }
    if own.gather_target.is_some() {
        flags |= OWN_GATHER;
    }
    if own.build_target.is_some() {
        flags |= OWN_BUILD;
    }
    if own.construction_remaining.is_some() {
        flags |= OWN_CONSTRUCTION;
    }
    if own.cargo.is_some() {
        flags |= OWN_CARGO;
    }
    if own.production.is_some() {
        flags |= OWN_PRODUCTION;
    }
    w.write_u8(flags)?;
    if let Some(target) = own.move_target {
        write_delta_pos(w, target, origin)?;
    }
    if let Some(target) = own.attack_target {
        w.write_u32(target.0)?;
    }
    if let Some(deposit) = own.gather_target {
        w.write_u32(deposit.0)?;
    }
    if let Some(target) = own.build_target {
        write_delta_pos(w, target, origin)?;
    }
    if let Some(remaining) = own.construction_remaining {
        w.write_u32(remaining)?;
    }
    if let Some((kind, amount)) = own.cargo {
        w.write_u8(resource_kind_byte(kind))?;
        w.write_u32(amount)?;
    }
    if let Some(queue) = &own.production {
        w.write_len(queue.len())?;
        for job in queue {
            w.write_u8(training_kind_byte(job.kind))?;
            w.write_u32(job.remaining)?;
        }
    }
    Ok(())
}

fn read_own_unit_state(r: &mut Reader<'_>, origin: IVec2) -> Result<OwnUnitState, NetError> {
    let flags = r.read_u8()?;
    if flags & !0x7f != 0 {
        return Err(NetError::invalid("unknown own-unit state flag"));
    }
    let move_target = if flags & OWN_MOVE != 0 {
        Some(read_delta_pos(r, origin)?)
    } else {
        None
    };
    let attack_target = if flags & OWN_ATTACK != 0 {
        Some(UnitId(r.read_u32()?))
    } else {
        None
    };
    let gather_target = if flags & OWN_GATHER != 0 {
        Some(DepositId(r.read_u32()?))
    } else {
        None
    };
    let build_target = if flags & OWN_BUILD != 0 {
        Some(read_delta_pos(r, origin)?)
    } else {
        None
    };
    let construction_remaining = if flags & OWN_CONSTRUCTION != 0 {
        Some(r.read_u32()?)
    } else {
        None
    };
    let cargo = if flags & OWN_CARGO != 0 {
        let kind = read_resource_kind(r.read_u8()?)?;
        let amount = r.read_u32()?;
        Some((kind, amount))
    } else {
        None
    };
    let production = if flags & OWN_PRODUCTION != 0 {
        let count = r.read_len(64)?;
        let mut queue = Vec::with_capacity(count.min(r.remaining().len() / 2));
        for _ in 0..count {
            let kind = read_training_kind(r.read_u8()?)?;
            let remaining = r.read_u32()?;
            queue.push(Training { kind, remaining });
        }
        Some(queue)
    } else {
        None
    };
    Ok(OwnUnitState {
        move_target,
        attack_target,
        gather_target,
        build_target,
        construction_remaining,
        cargo,
        production,
    })
}

const fn resource_kind_byte(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Wood => 0,
        ResourceKind::Gold => 1,
    }
}

fn read_resource_kind(byte: u8) -> Result<ResourceKind, NetError> {
    match byte {
        0 => Ok(ResourceKind::Wood),
        1 => Ok(ResourceKind::Gold),
        _ => Err(NetError::invalid("unknown resource kind")),
    }
}

const fn training_kind_byte(kind: TrainingKind) -> u8 {
    match kind {
        TrainingKind::Worker => 0,
        TrainingKind::RifleSquad => 1,
        TrainingKind::Scout => 2,
    }
}

fn read_training_kind(byte: u8) -> Result<TrainingKind, NetError> {
    match byte {
        0 => Ok(TrainingKind::Worker),
        1 => Ok(TrainingKind::RifleSquad),
        2 => Ok(TrainingKind::Scout),
        _ => Err(NetError::invalid("unknown training kind")),
    }
}

fn write_delta_pos(w: &mut Writer, pos: IVec2, base: IVec2) -> Result<(), NetError> {
    w.write_i64(i64::from(pos.x) - i64::from(base.x))?;
    w.write_i64(i64::from(pos.y) - i64::from(base.y))
}

fn read_delta_pos(r: &mut Reader<'_>, base: IVec2) -> Result<IVec2, NetError> {
    let dx = r.read_i64()?;
    let dy = r.read_i64()?;
    let x = i64::from(base.x)
        .checked_add(dx)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| NetError::invalid("coordinate out of range"))?;
    let y = i64::from(base.y)
        .checked_add(dy)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| NetError::invalid("coordinate out of range"))?;
    Ok(IVec2::new(x, y))
}

const fn zigzag_encode_i32(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)).cast_unsigned()
}

const fn zigzag_decode_i32(value: u32) -> i32 {
    ((value >> 1).cast_signed()) ^ (-((value & 1).cast_signed()))
}

const fn zigzag_encode_i64(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)).cast_unsigned()
}

const fn zigzag_decode_i64(value: u64) -> i64 {
    ((value >> 1).cast_signed()) ^ (-((value & 1).cast_signed()))
}

struct Writer {
    buf: Vec<u8>,
    limit: usize,
}

impl Writer {
    const fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
        }
    }

    fn reserve_extra(&self, extra: usize) -> Result<(), NetError> {
        if extra > self.limit.saturating_sub(self.buf.len()) {
            return Err(NetError::invalid(format!(
                "data exceeds {} bytes",
                self.limit
            )));
        }
        Ok(())
    }

    fn write_u8(&mut self, byte: u8) -> Result<(), NetError> {
        self.reserve_extra(1)?;
        self.buf.push(byte);
        Ok(())
    }

    fn write_bool(&mut self, value: bool) -> Result<(), NetError> {
        self.write_u8(u8::from(value))
    }

    fn write_u64(&mut self, mut value: u64) -> Result<(), NetError> {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.write_u8(byte)?;
            if value == 0 {
                return Ok(());
            }
        }
    }

    fn write_u32(&mut self, value: u32) -> Result<(), NetError> {
        self.write_u64(u64::from(value))
    }

    fn write_i32(&mut self, value: i32) -> Result<(), NetError> {
        self.write_u32(zigzag_encode_i32(value))
    }

    fn write_i64(&mut self, value: i64) -> Result<(), NetError> {
        self.write_u64(zigzag_encode_i64(value))
    }

    fn write_len(&mut self, len: usize) -> Result<(), NetError> {
        let len = u64::try_from(len).map_err(|_| NetError::invalid("length overflow"))?;
        self.write_u64(len)
    }

    fn write_str(&mut self, text: &str) -> Result<(), NetError> {
        self.write_len(text.len())?;
        self.reserve_extra(text.len())?;
        self.buf.extend_from_slice(text.as_bytes());
        Ok(())
    }

    fn write_ivec2(&mut self, pos: IVec2) -> Result<(), NetError> {
        self.write_i32(pos.x)?;
        self.write_i32(pos.y)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    const fn remaining(&self) -> &'a [u8] {
        self.bytes
    }

    fn finish(self) -> Result<(), NetError> {
        if !self.bytes.is_empty() {
            return Err(NetError::invalid("trailing bytes in wire message"));
        }
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, NetError> {
        let (&byte, rest) = self
            .bytes
            .split_first()
            .ok_or_else(|| NetError::invalid("truncated wire message"))?;
        self.bytes = rest;
        Ok(byte)
    }

    fn read_bool(&mut self) -> Result<bool, NetError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(NetError::invalid("invalid boolean byte")),
        }
    }

    fn read_u64(&mut self) -> Result<u64, NetError> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = self.read_u8()?;
            let part = u64::from(byte & 0x7f);
            if shift == 63 && part > 1 {
                return Err(NetError::invalid("varint overflow"));
            }
            value |= part << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(NetError::invalid("varint overflow"))
    }

    fn read_u32(&mut self) -> Result<u32, NetError> {
        u32::try_from(self.read_u64()?).map_err(|_| NetError::invalid("u32 varint overflow"))
    }

    fn read_i32(&mut self) -> Result<i32, NetError> {
        Ok(zigzag_decode_i32(self.read_u32()?))
    }

    fn read_i64(&mut self) -> Result<i64, NetError> {
        Ok(zigzag_decode_i64(self.read_u64()?))
    }

    fn read_len(&mut self, max: usize) -> Result<usize, NetError> {
        let value = usize::try_from(self.read_u64()?)
            .map_err(|_| NetError::invalid("length exceeds platform bounds"))?;
        if value > max {
            return Err(NetError::invalid("length exceeds protocol limit"));
        }
        Ok(value)
    }

    fn read_str(&mut self, max_bytes: usize) -> Result<String, NetError> {
        let len = self.read_len(max_bytes)?;
        let (slice, rest) = self
            .bytes
            .split_at_checked(len)
            .ok_or_else(|| NetError::invalid("truncated string in wire message"))?;
        self.bytes = rest;
        let text =
            str::from_utf8(slice).map_err(|_| NetError::invalid("wire string is not UTF-8"))?;
        Ok(text.to_owned())
    }

    fn read_ivec2(&mut self) -> Result<IVec2, NetError> {
        Ok(IVec2::new(self.read_i32()?, self.read_i32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_view_frame_and_simple_commands_are_minimal() {
        let quiet = ServerMessage::View {
            frame: ViewFrame {
                tick: 42,
                ..ViewFrame::default()
            },
        };
        let bytes = encode_server(&quiet).unwrap();
        assert_eq!(bytes.len(), 3, "quiet view frame should be 3 bytes");
        assert_eq!(decode_server(&bytes).unwrap(), quiet);

        assert_eq!(encode_client(&ClientMessage::Start).unwrap().len(), 1);
        assert_eq!(encode_client(&ClientMessage::Leave).unwrap().len(), 1);
        assert_eq!(
            encode_client(&ClientMessage::Ready { ready: true })
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn zigzag_and_varint_handle_extremes() {
        for value in [i32::MIN, -1000, -1, 0, 1, 1000, i32::MAX] {
            assert_eq!(zigzag_decode_i32(zigzag_encode_i32(value)), value);
        }
        for value in [i64::MIN, -1000, -1, 0, 1, 1000, i64::MAX] {
            assert_eq!(zigzag_decode_i64(zigzag_encode_i64(value)), value);
        }
    }

    #[test]
    fn bounded_inflate_rejects_corruption_truncation_and_length_mismatch() {
        let payload = b"hello world ".repeat(20);
        let compressed = compress_to_vec(&payload, 6);
        assert_eq!(
            inflate_bounded(payload.len(), &compressed, 1024).unwrap(),
            payload
        );
        assert!(inflate_bounded(payload.len() - 1, &compressed, 1024).is_err());
        assert!(inflate_bounded(payload.len() + 1, &compressed, 1024).is_err());
        assert!(inflate_bounded(payload.len(), &compressed[..compressed.len() - 1], 1024).is_err());
        let mut trailing = compressed.clone();
        trailing.push(0);
        assert!(inflate_bounded(payload.len(), &trailing, 1024).is_err());
        assert!(inflate_bounded(1025, &compressed, 1024).is_err());
    }
}
