//! `rust/src/xxhash/mod.rs`: the digest halves no caller can name.
//!
//! `low_64` is the crate-private narrowing every 64-bit digest takes from its
//! 128-bit sibling, so it is reached through `yggdryl::internals`; the two
//! widths it relates are public. Everything else a caller can observe lives in
//! `rust/tests/hashing/xxhash.rs`.

use yggdryl::internals::xxhash::low_64;
use yggdryl::xxhash::{xxh3, xxh128};

/// A deterministic payload of `length` bytes.
fn corpus(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index * 31 + 7) as u8).collect()
}

#[test]
fn xxh128_carries_xxh3_in_its_low_half_where_the_branches_agree() {
    // The reference shares its mixing between the two widths on exactly two
    // branches: 1-to-3 bytes, and the long path past the 240-byte cutoff.
    // Everywhere else - the empty input and the 4-to-240 branches - XXH3-128
    // folds a second accumulator that moves the low half, so the two widths
    // are different values and neither can be derived from the other.
    for length in [1, 2, 3, 241, 512, 4096] {
        let payload = corpus(length);
        assert_eq!(low_64(xxh128(&payload)), xxh3(&payload), "length {length}");
    }
    assert_eq!(low_64(xxh128(b"abc")), xxh3(b"abc"));

    for length in [0, 4, 8, 16, 128, 240] {
        let payload = corpus(length);
        assert_ne!(low_64(xxh128(&payload)), xxh3(&payload), "length {length}");
    }
}
