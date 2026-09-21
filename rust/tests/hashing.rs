//! Digest vocabulary integration tests.

#[path = "hashing/digest.rs"]
mod digest;
#[cfg(feature = "internals")]
#[path = "hashing/stable.rs"]
mod stable;
#[path = "hashing/txhash.rs"]
mod txhash;
#[path = "hashing/txhash_arrow.rs"]
mod txhash_arrow;
#[path = "hashing/xxhash.rs"]
mod xxhash;
#[path = "hashing/xxhash_arrow.rs"]
mod xxhash_arrow;
