//! Structural and canonical-display adapters for the stable XXH3-64 sink.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::DigestAlgorithm;
use crate::xxhash::Feed;

/// Hash canonical display output with the stable Yggdryl XXH3-64 contract.
///
/// The rendering is written into the sink rather than assembled: a short
/// one - a name, a term, a plan - is staged on the stack and hashed in one
/// shot, and a long one streams from the byte it outgrows the stage, so a
/// value whose canonical text is large costs no copy of it.
pub(crate) fn stable_hash_display(value: &impl fmt::Display) -> u64 {
    let mut hasher = StableHash::new();
    let result = fmt::write(&mut hasher, format_args!("{value}"));
    debug_assert!(result.is_ok(), "the stable hash sink is infallible");
    hasher.finish()
}

/// Hash a native structural [`Hash`] implementation with the stable sink.
pub(crate) fn stable_hash_of(value: &impl Hash) -> u64 {
    let mut hasher = StableHash::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// XXH3-64 behind explicit little-endian integer writes.
///
/// [`Hasher`]'s default `write_u8` through `write_usize` bodies use
/// native-endian bytes, so handing a bare XXH3 state to a [`Hash`]
/// implementation would make a stored hash disagree between a big-endian and a
/// little-endian machine. Overriding them is the whole reason this stays a
/// named type rather than an inline call.
struct StableHash(Feed);

impl StableHash {
    const fn new() -> Self {
        Self(Feed::new(DigestAlgorithm::Xxh3))
    }
}

impl fmt::Write for StableHash {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write(value.as_bytes());
        Ok(())
    }
}

impl Hasher for StableHash {
    fn finish(&self) -> u64 {
        self.0.as_u64()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i8(&mut self, value: i8) {
        self.write(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_i128(&mut self, value: i128) {
        self.write(&value.to_le_bytes());
    }

    fn write_isize(&mut self, value: isize) {
        self.write_i64(value as i64);
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hasher as _;

    use super::{StableHash, stable_hash_display, stable_hash_of};
    use crate::xxhash::xxh3;

    #[test]
    fn byte_and_display_hashing_agree() {
        // A str's Display output is its bytes, so hashing the rendering here
        // and hashing the bytes with `xxhash::xxh3` are the same function
        // reached two ways. That is the whole reason there is no second
        // byte-oriented spelling beside this one.
        for text in ["", "x", "fill 100 @ 187.23", "é—both\nlines"] {
            assert_eq!(xxh3(text.as_bytes()), stable_hash_display(&text));
        }
        // The published XXH3-64 vectors pin the contract.
        assert_eq!(stable_hash_display(&""), 0x2d06_8005_38d3_94c2);
        assert_eq!(stable_hash_display(&"abc"), 0x78af_5f94_892f_3950);
    }

    #[test]
    fn the_structural_sink_writes_little_endian_integers() {
        // Every `write_*` override is pinned against the explicit
        // little-endian bytes, so a big-endian target answers the same value a
        // little-endian one stored. Pointer widths always use 64-bit storage.
        macro_rules! check {
            ($method:ident, $value:expr, $bytes:expr) => {{
                let mut sink = StableHash::new();
                sink.$method($value);
                let expected = xxh3(&$bytes);
                assert_eq!(sink.finish(), expected);
                assert_eq!(stable_hash_of(&$value), expected);
            }};
        }

        check!(write_u8, 0x81_u8, 0x81_u8.to_le_bytes());
        check!(write_u16, 0x0102_u16, 0x0102_u16.to_le_bytes());
        check!(write_u32, 0x0102_0304_u32, 0x0102_0304_u32.to_le_bytes());
        check!(
            write_u64,
            0x0102_0304_0506_0708_u64,
            0x0102_0304_0506_0708_u64.to_le_bytes()
        );
        check!(write_u128, u128::MAX - 1, (u128::MAX - 1).to_le_bytes());
        check!(write_usize, 7_usize, 7_u64.to_le_bytes());
        check!(write_i8, -2_i8, (-2_i8).to_le_bytes());
        check!(write_i16, -257_i16, (-257_i16).to_le_bytes());
        check!(write_i32, -65_537_i32, (-65_537_i32).to_le_bytes());
        check!(write_i64, -2_i64, (-2_i64).to_le_bytes());
        check!(write_i128, i128::MIN + 1, (i128::MIN + 1).to_le_bytes());
        check!(write_isize, -7_isize, (-7_i64).to_le_bytes());

        let mut sink = StableHash::new();
        sink.write_u32(0x0102_0304);
        sink.write_i64(-2);
        sink.write_usize(7);
        let mut expected = Vec::new();
        expected.extend_from_slice(&0x0102_0304_u32.to_le_bytes());
        expected.extend_from_slice(&(-2_i64).to_le_bytes());
        expected.extend_from_slice(&7_u64.to_le_bytes());
        assert_eq!(sink.finish(), xxh3(&expected));
    }
}
