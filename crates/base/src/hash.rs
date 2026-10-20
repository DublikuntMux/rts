use std::hash::Hasher;

/// FNV-1a hasher for comparing simulation state between peers.
///
/// Integers are hashed as fixed-width little-endian bytes and `usize` as `u64`, so native and
/// `wasm32` builds produce the same hash.
pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for StateHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.write(&[n]);
    }

    fn write_u16(&mut self, n: u16) {
        self.write(&n.to_le_bytes());
    }

    fn write_u32(&mut self, n: u32) {
        self.write(&n.to_le_bytes());
    }

    fn write_u64(&mut self, n: u64) {
        self.write(&n.to_le_bytes());
    }

    fn write_u128(&mut self, n: u128) {
        self.write(&n.to_le_bytes());
    }

    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::*;

    #[test]
    fn matches_reference_fnv1a() {
        let mut hasher = StateHasher::default();
        hasher.write(b"a");
        assert_eq!(hasher.finish(), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn hashes_lengths_independently_of_pointer_width() {
        let mut collection = StateHasher::default();
        vec![7_u8].hash(&mut collection);
        let mut manual = StateHasher::default();
        manual.write_u64(1);
        manual.write_u8(7);
        assert_eq!(collection.finish(), manual.finish());
    }
}
