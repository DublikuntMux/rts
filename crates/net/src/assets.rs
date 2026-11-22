//! Server-to-client asset sync protocol for textures and localization files.
//!
//! Asset sync travels on its own low-priority bidirectional QUIC stream (`STREAM_ASSETS`),
//! using the same compact binary envelope as the game stream: header bit `7` marks raw-deflate
//! compression whenever compression shrinks the payload (e.g., `*.lang.ron` text shrinks ~4x,
//! while already-compressed `*.png` files stay uncompressed).

use std::collections::{BTreeMap, BTreeSet};

use miniz_oxide::{
    deflate::compress_to_vec,
    inflate::{
        TINFLStatus,
        core::{DecompressorOxide, decompress, inflate_flags},
    },
};
use serde::Deserialize;

use crate::{NetError, check_size, validate_version};

/// Maximum number of client assets a server can advertise.
pub const MAX_SYNC_ASSETS: usize = 256;
/// Maximum byte length of a relative asset path.
pub const MAX_ASSET_PATH_BYTES: usize = 128;
/// Maximum size of a single synced asset file (2 MiB).
pub const MAX_ASSET_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum combined size of all synced assets from one server (32 MiB).
pub const MAX_ASSET_PACK_BYTES: usize = 32 * 1024 * 1024;
/// Maximum encoded size of a single asset stream frame.
pub const MAX_ASSET_FRAME_BYTES: usize = MAX_ASSET_FILE_BYTES + 1024;
/// Maximum encoded size of a client asset stream request.
pub const MAX_ASSET_REQUEST_BYTES: usize = 65_536;

const COMPRESSED_BIT: u8 = 0x80;
const TAG_MASK: u8 = 0x7f;
const COMPRESS_THRESHOLD: usize = 64;

const AC_GET_MANIFEST: u8 = 0;
const AC_FETCH: u8 = 1;

const AS_MANIFEST: u8 = 0;
const AS_FILE: u8 = 1;
const AS_DONE: u8 = 2;
const AS_ERROR: u8 = 3;

/// Supported categories of server-provided client assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Texture,
    Locale,
}

/// Metadata for one client asset advertised by a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    pub path: String,
    pub sha256: [u8; 32],
    pub size: u32,
}

/// Client requests sent on `STREAM_ASSETS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetClientMessage {
    GetManifest { version: u32 },
    Fetch { paths: Vec<String> },
}

/// Server responses sent on `STREAM_ASSETS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetServerMessage {
    Manifest {
        entries: Vec<AssetEntry>,
    },
    File {
        path: String,
        sha256: [u8; 32],
        bytes: Vec<u8>,
    },
    Done,
    Error {
        message: String,
    },
}

/// Validates a relative client asset path and classifies its category.
///
/// Only normalized relative paths under `textures/**/*.png` or `locales/*.lang.ron` are allowed.
///
/// # Errors
/// Returns an error if the path is empty, too long, contains traversal or disallowed characters,
/// or falls outside the permitted asset categories.
pub fn validate_asset_path(path: &str) -> Result<AssetKind, NetError> {
    if path.is_empty() || path.len() > MAX_ASSET_PATH_BYTES || !path.is_ascii() {
        return Err(NetError::invalid("invalid asset path length or encoding"));
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains('\\') {
        return Err(NetError::invalid("asset path must be normalized relative"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 {
        return Err(NetError::invalid(
            "asset path must include a category folder",
        ));
    }
    for segment in &segments {
        if segment.is_empty()
            || *segment == "."
            || *segment == ".."
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(NetError::invalid("invalid segment in asset path"));
        }
    }
    let (Some(&category), Some(&file_name)) = (segments.first(), segments.last()) else {
        return Err(NetError::invalid(
            "asset path must include a category folder",
        ));
    };
    match category {
        "textures"
            if file_name
                .strip_suffix(".png")
                .is_some_and(|stem| !stem.is_empty()) =>
        {
            Ok(AssetKind::Texture)
        }
        "locales"
            if segments.len() == 2
                && file_name
                    .strip_suffix(".lang.ron")
                    .is_some_and(|stem| !stem.is_empty()) =>
        {
            Ok(AssetKind::Locale)
        }
        _ => Err(NetError::invalid(
            "asset path must be textures/**/*.png or locales/*.lang.ron",
        )),
    }
}

/// Validates that `bytes` is a valid UTF-8 `*.lang.ron` localization document.
///
/// # Errors
/// Returns an error if `bytes` is not valid UTF-8, fails RON parsing, or has invalid fields.
pub fn validate_locale_bytes(bytes: &[u8]) -> Result<(), NetError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LocaleHeader {
        code: String,
        name: String,
        strings: BTreeMap<String, String>,
    }

    let text = std::str::from_utf8(bytes)
        .map_err(|_| NetError::invalid("locale file is not valid UTF-8"))?;
    let parsed: LocaleHeader = ron::from_str(text)?;
    if parsed.code.len() < 2
        || parsed.code.len() > 16
        || parsed.name.trim().is_empty()
        || parsed.strings.len() > 4096
    {
        return Err(NetError::invalid("locale file failed field validation"));
    }
    Ok(())
}

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

#[expect(
    clippy::many_single_char_names,
    reason = "FIPS 180-4 SHA-256 working variables a..h"
)]
fn sha256_compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0_u32; 64];
    for (word, chunk) in schedule.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*chunk);
    }
    for i in 16..64 {
        let s0 = schedule[i - 15].rotate_right(7)
            ^ schedule[i - 15].rotate_right(18)
            ^ (schedule[i - 15] >> 3);
        let s1 = schedule[i - 2].rotate_right(17)
            ^ schedule[i - 2].rotate_right(19)
            ^ (schedule[i - 2] >> 10);
        schedule[i] = schedule[i - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(schedule[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Computes the standard FIPS-180-4 SHA-256 digest of `data` in pure Rust.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let (blocks, remainder) = data.as_chunks::<64>();
    for block in blocks {
        sha256_compress(&mut state, block);
    }
    let mut tail = [[0_u8; 64]; 2];
    let flat = tail.as_flattened_mut();
    flat[..remainder.len()].copy_from_slice(remainder);
    flat[remainder.len()] = 0x80;
    let two_blocks = remainder.len() >= 56;
    let end = if two_blocks { 128 } else { 64 };
    let bit_len = (data.len() as u64).wrapping_mul(8);
    flat[end - 8..end].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut state, &tail[0]);
    if two_blocks {
        sha256_compress(&mut state, &tail[1]);
    }

    let mut digest = [0_u8; 32];
    for (word, out) in state.iter().zip(digest.as_chunks_mut::<4>().0) {
        *out = word.to_be_bytes();
    }
    digest
}

/// Formats a SHA-256 digest as lowercase hexadecimal.
#[must_use]
pub fn hex_sha256(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Encodes a validated client asset request into compact binary wire bytes.
///
/// # Errors
/// Returns an error if validation fails or the encoded request exceeds [`MAX_ASSET_REQUEST_BYTES`].
pub fn encode_asset_client(message: &AssetClientMessage) -> Result<Vec<u8>, NetError> {
    validate_asset_client(message)?;
    let mut payload = Vec::new();
    let tag = match message {
        AssetClientMessage::GetManifest { version } => {
            write_varint(&mut payload, u64::from(*version));
            AC_GET_MANIFEST
        }
        AssetClientMessage::Fetch { paths } => {
            write_varint(&mut payload, paths.len() as u64);
            for path in paths {
                write_str(&mut payload, path);
            }
            AC_FETCH
        }
    };
    finish_envelope(tag, payload, MAX_ASSET_REQUEST_BYTES)
}

/// Decodes and validates a client asset request.
///
/// # Errors
/// Returns an error if the bytes are malformed, oversized, or fail validation.
pub fn decode_asset_client(bytes: &[u8]) -> Result<AssetClientMessage, NetError> {
    check_size(bytes.len(), MAX_ASSET_REQUEST_BYTES)?;
    let (tag, payload) = open_envelope(bytes, MAX_ASSET_REQUEST_BYTES)?;
    let mut cursor = payload.as_slice();
    let message = match tag {
        AC_GET_MANIFEST => AssetClientMessage::GetManifest {
            version: read_u32(&mut cursor)?,
        },
        AC_FETCH => {
            let count = read_len(&mut cursor, MAX_SYNC_ASSETS)?;
            let mut paths = Vec::with_capacity(count);
            for _ in 0..count {
                paths.push(read_str(&mut cursor, MAX_ASSET_PATH_BYTES)?);
            }
            AssetClientMessage::Fetch { paths }
        }
        _ => return Err(NetError::invalid("unknown asset client message tag")),
    };
    if !cursor.is_empty() {
        return Err(NetError::invalid("trailing bytes in asset client message"));
    }
    validate_asset_client(&message)?;
    Ok(message)
}

/// Encodes a validated server asset response into compact binary wire bytes.
///
/// # Errors
/// Returns an error if validation fails or the encoded frame exceeds [`MAX_ASSET_FRAME_BYTES`].
pub fn encode_asset_server(message: &AssetServerMessage) -> Result<Vec<u8>, NetError> {
    validate_asset_server(message)?;
    let mut payload = Vec::new();
    let tag = match message {
        AssetServerMessage::Manifest { entries } => {
            write_varint(&mut payload, entries.len() as u64);
            for entry in entries {
                write_str(&mut payload, &entry.path);
                payload.extend_from_slice(&entry.sha256);
                write_varint(&mut payload, u64::from(entry.size));
            }
            AS_MANIFEST
        }
        AssetServerMessage::File {
            path,
            sha256,
            bytes,
        } => {
            write_str(&mut payload, path);
            payload.extend_from_slice(sha256);
            write_varint(&mut payload, bytes.len() as u64);
            payload.extend_from_slice(bytes);
            AS_FILE
        }
        AssetServerMessage::Done => AS_DONE,
        AssetServerMessage::Error { message } => {
            write_str(&mut payload, message);
            AS_ERROR
        }
    };
    finish_envelope(tag, payload, MAX_ASSET_FRAME_BYTES)
}

/// Decodes and validates a server asset response.
///
/// # Errors
/// Returns an error if the bytes are malformed, oversized, or fail hash/path validation.
pub fn decode_asset_server(bytes: &[u8]) -> Result<AssetServerMessage, NetError> {
    check_size(bytes.len(), MAX_ASSET_FRAME_BYTES)?;
    let (tag, payload) = open_envelope(bytes, MAX_ASSET_FRAME_BYTES)?;
    let mut cursor = payload.as_slice();
    let message = match tag {
        AS_MANIFEST => {
            let count = read_len(&mut cursor, MAX_SYNC_ASSETS)?;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let path = read_str(&mut cursor, MAX_ASSET_PATH_BYTES)?;
                let digest = read_sha256(&mut cursor)?;
                let size = read_u32(&mut cursor)?;
                entries.push(AssetEntry {
                    path,
                    sha256: digest,
                    size,
                });
            }
            AssetServerMessage::Manifest { entries }
        }
        AS_FILE => {
            let path = read_str(&mut cursor, MAX_ASSET_PATH_BYTES)?;
            let digest = read_sha256(&mut cursor)?;
            let len = read_len(&mut cursor, MAX_ASSET_FILE_BYTES)?;
            let data = read_bytes(&mut cursor, len)?.to_vec();
            AssetServerMessage::File {
                path,
                sha256: digest,
                bytes: data,
            }
        }
        AS_DONE => AssetServerMessage::Done,
        AS_ERROR => AssetServerMessage::Error {
            message: read_str(&mut cursor, 512)?,
        },
        _ => return Err(NetError::invalid("unknown asset server message tag")),
    };
    if !cursor.is_empty() {
        return Err(NetError::invalid("trailing bytes in asset server message"));
    }
    validate_asset_server(&message)?;
    Ok(message)
}

fn validate_asset_client(message: &AssetClientMessage) -> Result<(), NetError> {
    match message {
        AssetClientMessage::GetManifest { version } => validate_version(*version),
        AssetClientMessage::Fetch { paths } => {
            if paths.len() > MAX_SYNC_ASSETS {
                return Err(NetError::invalid("too many requested asset paths"));
            }
            let mut seen = BTreeSet::new();
            for path in paths {
                validate_asset_path(path)?;
                if !seen.insert(path.as_str()) {
                    return Err(NetError::invalid("duplicate requested asset path"));
                }
            }
            Ok(())
        }
    }
}

fn validate_asset_server(message: &AssetServerMessage) -> Result<(), NetError> {
    match message {
        AssetServerMessage::Manifest { entries } => {
            if entries.len() > MAX_SYNC_ASSETS {
                return Err(NetError::invalid("too many manifest entries"));
            }
            let mut seen = BTreeSet::new();
            let mut total: usize = 0;
            for entry in entries {
                validate_asset_path(&entry.path)?;
                if !seen.insert(entry.path.as_str()) {
                    return Err(NetError::invalid("duplicate manifest asset path"));
                }
                let size = usize::try_from(entry.size)
                    .map_err(|_| NetError::invalid("asset size overflow"))?;
                if size == 0 || size > MAX_ASSET_FILE_BYTES {
                    return Err(NetError::invalid("asset size out of bounds"));
                }
                total = total
                    .checked_add(size)
                    .ok_or_else(|| NetError::invalid("asset pack size overflow"))?;
                if total > MAX_ASSET_PACK_BYTES {
                    return Err(NetError::invalid("asset pack exceeds size limit"));
                }
            }
            Ok(())
        }
        AssetServerMessage::File {
            path,
            sha256: expected,
            bytes,
        } => {
            validate_asset_path(path)?;
            if bytes.is_empty() || bytes.len() > MAX_ASSET_FILE_BYTES {
                return Err(NetError::invalid("asset file size out of bounds"));
            }
            if sha256(bytes) != *expected {
                return Err(NetError::invalid("asset SHA-256 mismatch"));
            }
            Ok(())
        }
        AssetServerMessage::Done | AssetServerMessage::Error { .. } => Ok(()),
    }
}

fn finish_envelope(tag: u8, payload: Vec<u8>, limit: usize) -> Result<Vec<u8>, NetError> {
    if payload.len() >= COMPRESS_THRESHOLD {
        let compressed = compress_to_vec(&payload, 6);
        if compressed.len() < payload.len() {
            check_size(1 + compressed.len(), limit)?;
            let mut out = Vec::with_capacity(1 + compressed.len());
            out.push(tag | COMPRESSED_BIT);
            out.extend_from_slice(&compressed);
            return Ok(out);
        }
    }
    check_size(1 + payload.len(), limit)?;
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(tag);
    out.extend_from_slice(&payload);
    Ok(out)
}

fn open_envelope(bytes: &[u8], max_uncompressed: usize) -> Result<(u8, Vec<u8>), NetError> {
    let (&header, rest) = bytes
        .split_first()
        .ok_or_else(|| NetError::invalid("empty asset wire message"))?;
    let tag = header & TAG_MASK;
    if header & COMPRESSED_BIT != 0 {
        Ok((tag, inflate_bounded(rest, max_uncompressed)?))
    } else {
        Ok((tag, rest.to_vec()))
    }
}

fn inflate_bounded(input: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    let mut decompressor = DecompressorOxide::new();
    let mut output = vec![0_u8; input.len().saturating_mul(2).clamp(256, limit)];
    let mut in_pos = 0;
    let mut out_pos = 0;
    loop {
        let (status, consumed, written) = decompress(
            &mut decompressor,
            &input[in_pos..],
            &mut output,
            out_pos,
            inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
        );
        in_pos += consumed;
        out_pos += written;
        match status {
            TINFLStatus::Done => {
                if in_pos != input.len() {
                    return Err(NetError::invalid("trailing compressed data"));
                }
                output.truncate(out_pos);
                return Ok(output);
            }
            TINFLStatus::HasMoreOutput => {
                if output.len() >= limit {
                    return Err(NetError::invalid(format!(
                        "decompressed asset message exceeds {limit} bytes"
                    )));
                }
                let next = output.len().saturating_mul(2).min(limit);
                output.resize(next, 0);
            }
            _ => return Err(NetError::invalid("corrupt compressed asset message")),
        }
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    write_varint(out, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
}

fn read_varint(cursor: &mut &[u8]) -> Result<u64, NetError> {
    let mut result = 0_u64;
    let mut shift = 0_u32;
    loop {
        let (&byte, rest) = cursor
            .split_first()
            .ok_or_else(|| NetError::invalid("unexpected end of asset frame"))?;
        *cursor = rest;
        if shift >= 64 || (shift == 63 && byte > 1) {
            return Err(NetError::invalid("varint overflow"));
        }
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32, NetError> {
    u32::try_from(read_varint(cursor)?).map_err(|_| NetError::invalid("u32 overflow"))
}

fn read_len(cursor: &mut &[u8], max: usize) -> Result<usize, NetError> {
    let len =
        usize::try_from(read_varint(cursor)?).map_err(|_| NetError::invalid("length overflow"))?;
    if len > max {
        return Err(NetError::invalid("length exceeds asset protocol limit"));
    }
    Ok(len)
}

fn read_bytes<'a>(cursor: &mut &'a [u8], len: usize) -> Result<&'a [u8], NetError> {
    if cursor.len() < len {
        return Err(NetError::invalid("truncated asset payload"));
    }
    let (head, tail) = cursor.split_at(len);
    *cursor = tail;
    Ok(head)
}

fn read_sha256(cursor: &mut &[u8]) -> Result<[u8; 32], NetError> {
    let bytes = read_bytes(cursor, 32)?;
    let mut out = [0_u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn read_str(cursor: &mut &[u8], max_bytes: usize) -> Result<String, NetError> {
    let len = read_len(cursor, max_bytes)?;
    let bytes = read_bytes(cursor, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| NetError::invalid("invalid UTF-8 in asset message"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PROTOCOL_VERSION;

    #[test]
    fn sha256_matches_nist_vectors() {
        assert_eq!(
            hex_sha256(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_sha256(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_sha256(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn asset_path_validation_accepts_textures_and_locales_only() {
        assert_eq!(
            validate_asset_path("textures/units/soldier.png").unwrap(),
            AssetKind::Texture
        );
        assert_eq!(
            validate_asset_path("textures/terrain/grass.png").unwrap(),
            AssetKind::Texture
        );
        assert_eq!(
            validate_asset_path("locales/fr.lang.ron").unwrap(),
            AssetKind::Locale
        );
        for bad in [
            "",
            "/textures/a.png",
            "textures/../a.png",
            "textures/a.jpg",
            "textures/.png",
            "textures\\a.png",
            "locales/sub/en.lang.ron",
            "locales/.lang.ron",
            "data/core.rules.ron",
            "fonts/Exo2.ttf",
        ] {
            assert!(validate_asset_path(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn asset_messages_roundtrip_and_compress_text_assets() {
        let req = AssetClientMessage::GetManifest {
            version: PROTOCOL_VERSION,
        };
        let encoded = encode_asset_client(&req).unwrap();
        assert_eq!(decode_asset_client(&encoded).unwrap(), req);

        let fetch = AssetClientMessage::Fetch {
            paths: vec![
                "locales/fr.lang.ron".into(),
                "textures/units/custom.png".into(),
            ],
        };
        let encoded = encode_asset_client(&fetch).unwrap();
        assert_eq!(decode_asset_client(&encoded).unwrap(), fetch);

        let locale_bytes = include_bytes!("../../../assets/locales/en.lang.ron").to_vec();
        let digest = sha256(&locale_bytes);
        let file = AssetServerMessage::File {
            path: "locales/en.lang.ron".into(),
            sha256: digest,
            bytes: locale_bytes.clone(),
        };
        let encoded = encode_asset_server(&file).unwrap();
        assert!(
            encoded.len() < locale_bytes.len() / 2,
            "locale text should deflate cleanly: {} vs {}",
            encoded.len(),
            locale_bytes.len()
        );
        assert_eq!(decode_asset_server(&encoded).unwrap(), file);

        let manifest = AssetServerMessage::Manifest {
            entries: vec![AssetEntry {
                path: "locales/en.lang.ron".into(),
                sha256: digest,
                size: u32::try_from(locale_bytes.len()).unwrap(),
            }],
        };
        let encoded = encode_asset_server(&manifest).unwrap();
        assert_eq!(decode_asset_server(&encoded).unwrap(), manifest);

        let done = encode_asset_server(&AssetServerMessage::Done).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(
            decode_asset_server(&done).unwrap(),
            AssetServerMessage::Done
        );
    }
}
