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
//! the split of a payload never changes its digest. That is what lets a
//! message spliced from two spans - a record with the row header removed from
//! the middle of a line - hash the same as the equivalent joined string,
//! without ever building the join. A hash that depended on where the row
//! header sat would be a silent correctness bug.
//!
//! ```
//! use yggdryl::xxhash::{Xxh3_64, xxh3_64};
//!
//! let mut state = Xxh3_64::new();
//! state.write_bytes(b"fill ");
//! state.write_bytes(b"100");
//! assert_eq!(state.as_u64(), xxh3_64(b"fill 100"));
//!
//! // An empty chunk contributes nothing, wherever it sits.
//! let mut padded = Xxh3_64::new();
//! for chunk in [b"".as_slice(), b"fill 100".as_slice(), b"".as_slice()] {
//!     padded.write_bytes(chunk);
//! }
//! assert_eq!(padded.as_u64(), xxh3_64(b"fill 100"));
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
//! [`Scalar::write_bytes`](crate::Scalar::write_bytes) extends the same
//! streaming feed to any value, and is what `stable_hash` and every row digest
//! read.
//!
//! # Custom secrets past the 240-byte cutoff
//!
//! XXH3 consults a custom secret only for inputs longer than 240 bytes. At or
//! below that length the algorithm uses its derived secret and the seed, which
//! is what [`SECRET_MINIMUM_LENGTH`]'s own specification says of the
//! seed-and-secret family and what keeps a one-shot and a streaming state
//! answering one value for the same bytes. A short secret is still rejected by
//! length whatever the payload, so a secret is never *silently* the wrong one -
//! but a caller hashing short values with a secret is hashing them with the
//! default secret, by the protocol's design.
//!
//! ```
//! use yggdryl::xxhash::{self, SECRET_MINIMUM_LENGTH};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let secret = vec![0x5a_u8; SECRET_MINIMUM_LENGTH];
//! let short = b"AAPL";
//! let long = vec![0x11_u8; 241];
//!
//! assert_eq!(xxhash::xxh3_64_with_secret(short, &secret)?, xxhash::xxh3_64(short));
//! assert_ne!(xxhash::xxh3_64_with_secret(&long, &secret)?, xxhash::xxh3_64(&long));
//! # Ok(())
//! # }
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
//! other reader would find them. [`crate::media::iceberg`] never calls this module for
//! partitioning.

#[cfg(feature = "arrow")]
pub mod arrow;
mod handle;
mod scalar;
mod secret;
mod state;
pub(crate) mod stream;

pub use handle::Hashed;
pub use scalar::ValueBytes;
pub use secret::SECRET_MINIMUM_LENGTH;
pub use state::{Xxh3_64, Xxh3_128, Xxh32, Xxh64};
pub use stream::{DigestReader, DigestWriter};

pub(crate) use state::{low_32, low_64};

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

/// Wrap a reader so the bytes it yields are digested as they pass.
///
/// This is the codings' `reader`/`writer` pair for digests: a caller who is
/// already moving a payload hashes it in the same pass rather than reading it
/// twice.
pub fn reader<R: std::io::Read>(source: R, algorithm: DigestAlgorithm) -> DigestReader<R> {
    DigestReader::new(source, algorithm)
}

/// Wrap a writer so the bytes written through it are digested.
pub fn writer<W: std::io::Write>(target: W, algorithm: DigestAlgorithm) -> DigestWriter<W> {
    DigestWriter::new(target, algorithm)
}

#[cfg(test)]
mod tests;
