//! `rust/src/hashing/stable.rs`: the stable sink no caller can name.
//!
//! `StableHash` is the XXH3-64 sink every `stable_hash` in the project writes
//! into, and its little-endian integer overrides are what keep a stored hash
//! the same value on a big-endian machine. Neither the type nor the two
//! `pub(crate)` entry points is reachable from `yggdryl::`, so they are
//! reached through `yggdryl::internals`; the digests a caller can observe are
//! pinned in `rust/tests/xxhash/mod_.rs`.

#[cfg(feature = "internals")]
mod internal {
    use std::hash::Hasher as _;

    use yggdryl::internals::hashing_stable::{
        stable_hash_display, stable_hash_of, stable_hash_sink,
    };
    use yggdryl::xxhash::xxh3;

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
                let mut sink = stable_hash_sink();
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

        let mut sink = stable_hash_sink();
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

mod txhash {
    use yggdryl::txhash::txh3;

    use yggdryl::TimeUnit;

    const INSTANT: i64 = 1_700_000_000_000_000;

    #[test]
    fn stable_hash_is_deterministic_and_value_wide() {
        let value = txh3(b"AAPL", INSTANT);
        assert_eq!(value.stable_hash(), value.stable_hash());
        assert_ne!(
            value.stable_hash(),
            txh3(b"AAPL", INSTANT + 1).stable_hash()
        );
        assert_ne!(
            value.stable_hash(),
            value.with_unit(TimeUnit::Second).unwrap().stable_hash()
        );
    }
}
