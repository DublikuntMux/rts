//! Binary setup framing. Replays retain ordinary, inspectable RON documents.

use miniz_oxide::{
    deflate::compress_to_vec_zlib,
    inflate::{
        TINFLStatus,
        core::{DecompressorOxide, decompress, inflate_flags},
    },
};

use super::{MAX_SETUP_BYTES, MatchSetup};
use crate::{
    NetError, PROTOCOL_VERSION, check_size, minify::minify_ron, validate_players, validate_version,
};

const TAG: &[u8; 4] = b"RTSS";
const HEADER_BYTES: usize = 33;
// Zlib's worst-case expansion is much smaller than this, even on incompressible input.
const MAX_COMPRESSED_BYTES: usize = MAX_SETUP_BYTES * 2;
pub const MAX_FRAME_BYTES: usize = HEADER_BYTES + MAX_COMPRESSED_BYTES * 2;

struct Document {
    len: u32,
    zlib: Vec<u8>,
}

impl Document {
    fn compress(source: &str) -> Result<Self, NetError> {
        check_size(source.len(), MAX_SETUP_BYTES)?;
        let compact = minify_ron(source);
        let zlib = compress_to_vec_zlib(compact.as_bytes(), 6);
        check_size(zlib.len(), MAX_COMPRESSED_BYTES)?;
        Ok(Self {
            len: u32::try_from(compact.len())
                .map_err(|_| NetError::invalid("minified RON exceeds the size limit"))?,
            zlib,
        })
    }
}

fn inflate(len: u32, compressed: &[u8]) -> Result<String, NetError> {
    let len = usize::try_from(len)
        .map_err(|_| NetError::invalid("decompressed RON exceeds the size limit"))?;
    check_size(len, MAX_SETUP_BYTES)?;
    check_size(compressed.len(), MAX_COMPRESSED_BYTES)?;
    let mut output = vec![0; len];
    let (status, consumed, written) = decompress(
        &mut Box::<DecompressorOxide>::default(),
        compressed,
        &mut output,
        0,
        inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
            | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    if status != TINFLStatus::Done || consumed != compressed.len() || written != len {
        return Err(NetError::invalid("invalid compressed setup document"));
    }
    String::from_utf8(output).map_err(|_| NetError::invalid("setup document is not valid UTF-8"))
}

pub fn encode(setup: &MatchSetup) -> Result<Vec<u8>, NetError> {
    let rules = Document::compress(&setup.rules)?;
    let map = Document::compress(&setup.map)?;
    let mut frame = Vec::with_capacity(HEADER_BYTES + rules.zlib.len() + map.zlib.len());
    frame.extend_from_slice(TAG);
    frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    frame.extend_from_slice(&setup.seed.to_le_bytes());
    frame.push(setup.players);
    for document in [&rules, &map] {
        frame.extend_from_slice(&document.len.to_le_bytes());
        let len = u32::try_from(document.zlib.len())
            .map_err(|_| NetError::invalid("compressed RON exceeds the size limit"))?;
        frame.extend_from_slice(&len.to_le_bytes());
    }
    frame.extend_from_slice(&rules.zlib);
    frame.extend_from_slice(&map.zlib);
    Ok(frame)
}

pub fn decode(bytes: &[u8]) -> Result<MatchSetup, NetError> {
    check_size(bytes.len(), MAX_FRAME_BYTES)?;
    let (mut header, payload) = bytes
        .split_at_checked(HEADER_BYTES)
        .ok_or_else(|| NetError::invalid("truncated binary setup header"))?;
    if take::<4>(&mut header)? != *TAG {
        return Err(NetError::invalid("unknown binary server message"));
    }
    validate_version(u32::from_le_bytes(take(&mut header)?))?;
    let seed = u64::from_le_bytes(take(&mut header)?);
    let players = take::<1>(&mut header)?[0];
    validate_players(players)?;
    let rules_len = u32::from_le_bytes(take(&mut header)?);
    let rules_compressed_len = read_length(&mut header, MAX_COMPRESSED_BYTES)?;
    let map_len = u32::from_le_bytes(take(&mut header)?);
    let map_compressed_len = read_length(&mut header, MAX_COMPRESSED_BYTES)?;
    for len in [rules_len, map_len] {
        check_size(usize::try_from(len).unwrap_or(usize::MAX), MAX_SETUP_BYTES)?;
    }
    let (rules, map) = payload
        .split_at_checked(rules_compressed_len)
        .ok_or_else(|| NetError::invalid("truncated binary setup payload"))?;
    if map.len() != map_compressed_len {
        return Err(NetError::invalid("binary setup payload length mismatch"));
    }
    Ok(MatchSetup {
        rules: inflate(rules_len, rules)?,
        map: inflate(map_len, map)?,
        seed,
        players,
    })
}

fn take<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], NetError> {
    let (value, rest) = bytes
        .split_at_checked(N)
        .ok_or_else(|| NetError::invalid("truncated binary setup header"))?;
    let value = value
        .try_into()
        .map_err(|_| NetError::invalid("invalid binary setup header"))?;
    *bytes = rest;
    Ok(value)
}

fn read_length(header: &mut &[u8], limit: usize) -> Result<usize, NetError> {
    let len = usize::try_from(u32::from_le_bytes(take(header)?))
        .map_err(|_| NetError::invalid("compressed RON exceeds the size limit"))?;
    check_size(len, limit)?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_inflate_checks_exact_length_utf8_and_complete_stream() {
        for bytes in [b"".as_slice(), b"hello", "\u{1f680}".as_bytes()] {
            assert_eq!(
                inflate(
                    u32::try_from(bytes.len()).unwrap(),
                    &compress_to_vec_zlib(bytes, 6)
                )
                .unwrap()
                .as_bytes(),
                bytes
            );
        }
        let compressed = compress_to_vec_zlib(b"hello", 6);
        for len in [0, 4, 6, u32::MAX] {
            assert!(inflate(len, &compressed).is_err());
        }
        assert!(inflate(1, &compress_to_vec_zlib(&[0xff], 6)).is_err());
        for end in 0..compressed.len() {
            assert!(
                inflate(5, &compressed[..end]).is_err(),
                "accepted truncation {end}"
            );
        }
        let mut trailing = compressed.clone();
        trailing.push(0);
        assert!(inflate(5, &trailing).is_err());
        let mut corrupt = compressed;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(inflate(5, &corrupt).is_err());
    }

    #[test]
    fn declared_size_and_compressed_input_limits_prevent_unbounded_expansion() {
        let mut bytes = vec![b'x'; MAX_SETUP_BYTES];
        assert_eq!(
            inflate(
                u32::try_from(bytes.len()).unwrap(),
                &compress_to_vec_zlib(&bytes, 6)
            )
            .unwrap()
            .len(),
            MAX_SETUP_BYTES
        );
        bytes.push(b'x');
        assert!(
            inflate(
                u32::try_from(MAX_SETUP_BYTES).unwrap(),
                &compress_to_vec_zlib(&bytes, 6)
            )
            .is_err()
        );
        assert!(inflate(u32::try_from(MAX_SETUP_BYTES + 1).unwrap(), &[]).is_err());
        assert!(inflate(1, &vec![0; MAX_COMPRESSED_BYTES + 1]).is_err());
    }
}
