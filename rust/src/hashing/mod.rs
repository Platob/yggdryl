//! Byte, value and time-coupled digests under one implementation owner.
//!
//! [`xxhash`] implements the four xxHash algorithms, their resumable states,
//! the canonical scalar/Arrow byte feed, and hashing handles. [`txhash`]
//! couples a digest with an explicit Unix count and resolution. The shared
//! dispatch vocabulary remains [`crate::DigestAlgorithm`] and [`crate::Digest`].
//!
//! A digest identifies the bytes or values its owner selects, not an assumed
//! schema. For example, [`crate::FixMsg::digest`] identifies the arrival body
//! independently of its delivery envelope. A message's `currhashcode` is the
//! XXH3-64 of what its event states and the canonical named content behind
//! it, coupled with its instant into the UUIDv7 that is its identity; its
//! `crosshashcode` is the XXH3-64 of the chain identifier it shares. These
//! FIX recipes reuse the shared algorithms; they do not define another hash
//! engine. A raw
//! [`txhash::TxHash`] is a time/digest pair, not an RFC UUID.
//! These hashes detect accidental differences; they are not cryptographic
//! integrity checks and cannot guarantee distinct identities.
//!
//! Scalar and streaming operations need no Arrow runtime. Each child module
//! gates only its Arrow implementation behind the `arrow` feature.

mod stable;
pub mod txhash;
pub mod xxhash;

pub(crate) use stable::{stable_hash_display, stable_hash_of};
