//! XXH32, XXH64, XXH3-64, and XXH3-128 over bytes, values, and handles.
//!
//! One-shot digests answer the native width with no wrapper around the
//! number. [`DigestAlgorithm`] is the runtime dispatcher when the algorithm is
//! a value rather than a call, and [`Digest`] is the answer when a caller
//! wants the algorithm carried with it.
//!
//! ```
//! use yggdryl::xxhash;
//!
//! assert_eq!(xxhash::xxh3_64(b"abc"), 0x78af_5f94_892f_3950);
//! assert_eq!(xxhash::xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
//! ```
//!
//! The four resumable states - [`Xxh32`], [`Xxh64`], [`Xxh3_64`],
//! [`Xxh3_128`] - feed bytes and readers and answer without being consumed, so
//! the split of a payload never changes its digest:
//!
//! ```
//! use yggdryl::xxhash::{Xxh3_64, xxh3_64};
//!
//! let payload = b"symbol,price\nAAPL,187.23\n";
//! for split in [1, 7, payload.len()] {
//!     let mut state = Xxh3_64::new();
//!     for chunk in payload.chunks(split) {
//!         state.write_bytes(chunk);
//!     }
//!     assert_eq!(state.as_u64(), xxh3_64(payload));
//! }
//! ```
//!
//! # This is not a cryptographic hash
//!
//! xxHash is a fast non-cryptographic hash. A [`Digest`] detects accidental
//! change - a truncated upload, a stale cache entry, a duplicated row - and
//! nothing more. Never use one as an integrity check against an adversary who
//! is allowed to choose the input, and never as a password or signature
//! primitive.
//!
//! # This is not Iceberg's bucket transform
//!
//! The Iceberg specification mandates murmur3 x86_32 for `bucket[N]`. A
//! partition computed with xxHash would place rows in the wrong buckets and no
//! other reader would find them. [`crate::iceberg`] never calls this module for
//! partitioning.

mod secret;
mod state;

pub use secret::SECRET_MINIMUM_LENGTH;
pub use state::{Xxh3_64, Xxh3_128, Xxh32, Xxh64};

pub(crate) use state::low_64;

use crate::{Digest, DigestAlgorithm, Result};

/// Digest a complete buffer with XXH32.
pub fn xxh32(input: &[u8]) -> u32 {
    xxh32_with_seed(input, 0)
}

/// Digest a complete buffer with XXH32 under an explicit seed.
///
/// The input comes first and the seed second, here and in every other seeded
/// entry point, so the payload stays the subject of the call.
pub fn xxh32_with_seed(input: &[u8], seed: u32) -> u32 {
    twox_hash::xxhash32::Hasher::oneshot(seed, input)
}

/// Digest a complete buffer with XXH64.
pub fn xxh64(input: &[u8]) -> u64 {
    xxh64_with_seed(input, 0)
}

/// Digest a complete buffer with XXH64 under an explicit seed.
pub fn xxh64_with_seed(input: &[u8], seed: u64) -> u64 {
    twox_hash::xxhash64::Hasher::oneshot(seed, input)
}

/// Digest a complete buffer with XXH3, answering 64 bits.
pub fn xxh3_64(input: &[u8]) -> u64 {
    twox_hash::xxhash3_64::Hasher::oneshot(input)
}

/// Digest a complete buffer with XXH3-64 under an explicit seed.
pub fn xxh3_64_with_seed(input: &[u8], seed: u64) -> u64 {
    twox_hash::xxhash3_64::Hasher::oneshot_with_seed(seed, input)
}

/// Digest a complete buffer with XXH3-64 under a custom secret.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
/// [`SECRET_MINIMUM_LENGTH`].
pub fn xxh3_64_with_secret(input: &[u8], secret: &[u8]) -> Result<u64> {
    xxh3_64_with_seed_and_secret(input, 0, secret)
}

/// Digest a complete buffer with XXH3-64 under a custom seed and secret.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
/// [`SECRET_MINIMUM_LENGTH`].
pub fn xxh3_64_with_seed_and_secret(input: &[u8], seed: u64, secret: &[u8]) -> Result<u64> {
    // The dependency only consults the secret past its 240-byte cutoff, so a
    // short secret would pass unnoticed on a short input. Validating here
    // makes a secret either accepted or rejected, never conditionally used.
    secret::validate(DigestAlgorithm::Xxh3_64, secret)?;
    match twox_hash::xxhash3_64::Hasher::oneshot_with_seed_and_secret(seed, secret, input) {
        Ok(digest) => Ok(digest),
        Err(_) => unreachable!("the secret was validated first"),
    }
}

/// Digest a complete buffer with XXH3, answering 128 bits.
pub fn xxh3_128(input: &[u8]) -> u128 {
    twox_hash::xxhash3_128::Hasher::oneshot(input)
}

/// Digest a complete buffer with XXH3-128 under an explicit seed.
pub fn xxh3_128_with_seed(input: &[u8], seed: u64) -> u128 {
    twox_hash::xxhash3_128::Hasher::oneshot_with_seed(seed, input)
}

/// Digest a complete buffer with XXH3-128 under a custom secret.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
/// [`SECRET_MINIMUM_LENGTH`].
pub fn xxh3_128_with_secret(input: &[u8], secret: &[u8]) -> Result<u128> {
    xxh3_128_with_seed_and_secret(input, 0, secret)
}

/// Digest a complete buffer with XXH3-128 under a custom seed and secret.
///
/// # Errors
///
/// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
/// [`SECRET_MINIMUM_LENGTH`].
pub fn xxh3_128_with_seed_and_secret(input: &[u8], seed: u64, secret: &[u8]) -> Result<u128> {
    secret::validate(DigestAlgorithm::Xxh3_128, secret)?;
    match twox_hash::xxhash3_128::Hasher::oneshot_with_seed_and_secret(seed, secret, input) {
        Ok(digest) => Ok(digest),
        Err(_) => unreachable!("the secret was validated first"),
    }
}

/// Digest a complete buffer, carrying the algorithm with the answer.
///
/// ```
/// use yggdryl::{DigestAlgorithm, xxhash};
///
/// let digest = xxhash::digest(b"abc", DigestAlgorithm::Xxh3_128);
/// assert_eq!(digest.to_string(), "xxh3-128:06b05ab6733a618578af5f94892f3950");
/// ```
pub fn digest(input: &[u8], algorithm: DigestAlgorithm) -> Digest {
    algorithm.digest(input)
}

#[cfg(test)]
mod tests;
