//! JavaScript views over the byte/value digests and the time-coupled digests.
//!
//! The loader publishes both families as one frozen `hashing` namespace,
//! `{ xxhash, txhash }`, over the classes and private native halves declared
//! here.

pub(crate) mod txhash;
pub(crate) mod xxhash;
