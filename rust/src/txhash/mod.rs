//! An instant coupled with an xxHash digest, in one sortable value.
//!
//! A [`TxHash`] is a unix count followed by a [`Digest`](crate::Digest): the
//! instant first,
//! big-endian, so the canonical bytes sort by time and then by content; the
//! digest after it, at its algorithm's exact width. The instant is always
//! UTC and counted in microseconds unless a resolution is named, and the
//! digest is whatever [`crate::xxhash`] answers for the same bytes - this
//! module defines no second hash, only the coupling.
//!
//! ```
//! use yggdryl::{DigestAlgorithm, txhash, xxhash};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let value = txhash::txh3(b"AAPL", 1_700_000_000_000_000);
//! assert_eq!(value.unix(), 1_700_000_000_000_000);
//! assert_eq!(value.digest(), DigestAlgorithm::Xxh3.digest(b"AAPL"));
//!
//! // Sixteen bytes: the instant, then the digest.
//! let bytes = value.into_bytes();
//! assert_eq!(bytes.len(), 16);
//! assert_eq!(&bytes[8..], &xxhash::xxh3(b"AAPL").to_be_bytes());
//! assert_eq!(txhash::TxHash::from_bytes(value.unit(), value.algorithm(), &bytes)?, value);
//! # Ok(())
//! # }
//! ```
//!
//! [`TxHasher`] holds one configuration - resolution, algorithm, seed,
//! secret - for every value, row, or batch it answers. The [`arrow`] module
//! answers whole columns: a timestamp, date, or integer column beside a batch
//! becomes one `fixed_size_binary` column of coupled values, and a digest
//! holder declaring `digest:time` is filled the same way by every
//! `apply_arrow_batch`.
//!
//! # What the instant means
//!
//! A unix count is an instant, never a wall clock: a zoned datetime already
//! counts from the epoch, and a naive one is read as if it were UTC, the
//! convention every naive timestamp in the project follows. Restating a
//! count at a coarser resolution floors it, so two instants keep their order
//! across resolutions and a coupled value is the bucket its instant falls in.
//!
//! # This is not a cryptographic hash
//!
//! The digest half is xxHash, and everything [`crate::xxhash`] says of it
//! holds here: it detects accidental change and withstands no adversary.

#[cfg(feature = "arrow")]
pub mod arrow;
mod field;
mod hasher;
mod scalar;
mod time;
mod value;

pub use hasher::TxHasher;
pub use time::{DEFAULT_UNIT, restate_unix, unix_from_scalar, unix_now};
pub use value::{TxHash, TxHashBytes, UNIX_WIDTH, algorithm_of_width, dtype, width};

pub(crate) use field::{
    DIGEST_TIME_KEY, DIGEST_UNIT_KEY, canonicalize_digest_unit, coupled_holder_accepts,
    coupled_holder_algorithm, expected_coupled_dtype, validate_digest_time,
};

use crate::DigestAlgorithm;

/// Couple a microsecond instant with XXH32 of a complete buffer.
pub fn txh32(input: &[u8], unix: i64) -> TxHash {
    digest(input, unix, DigestAlgorithm::Xxh32)
}

/// Couple a microsecond instant with XXH64 of a complete buffer.
pub fn txh64(input: &[u8], unix: i64) -> TxHash {
    digest(input, unix, DigestAlgorithm::Xxh64)
}

/// Couple a microsecond instant with XXH3-64 of a complete buffer.
pub fn txh3(input: &[u8], unix: i64) -> TxHash {
    digest(input, unix, DigestAlgorithm::Xxh3)
}

/// Couple a microsecond instant with XXH3-128 of a complete buffer.
pub fn txh128(input: &[u8], unix: i64) -> TxHash {
    digest(input, unix, DigestAlgorithm::Xxh128)
}

/// Couple a microsecond instant with a complete buffer's digest.
///
/// The input comes first and the instant second, as the seed does in every
/// seeded digest entry point, so the payload stays the subject of the call.
///
/// ```
/// use yggdryl::{DigestAlgorithm, txhash};
///
/// let value = txhash::digest(b"abc", 0, DigestAlgorithm::Xxh128);
/// assert_eq!(value.to_string(), "0@us:xxh3-128:06b05ab6733a618578af5f94892f3950");
/// ```
pub fn digest(input: &[u8], unix: i64, algorithm: DigestAlgorithm) -> TxHash {
    TxHash::new(unix, algorithm.digest(input))
}

#[cfg(test)]
mod tests;
