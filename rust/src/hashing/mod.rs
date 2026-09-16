//! Byte, value and time-coupled digests under one implementation owner.
//!
//! [`xxhash`] implements the four xxHash algorithms, their resumable states,
//! the canonical scalar/Arrow byte feed, and hashing handles. [`txhash`]
//! couples a digest with an explicit Unix count and resolution. The shared
//! dispatch vocabulary remains [`crate::DigestAlgorithm`] and [`crate::Digest`].
//!
//! A digest identifies the bytes or values its owner selects, not an assumed
//! schema. For example, [`crate::FixMsg::digest`] identifies the arrival body
//! independently of its delivery envelope. [`crate::FixMsg::msgphash`] hashes exact
//! chain-code bytes; [`crate::FixMsg::msghash`] couples updatedat nanoseconds with
//! the canonical named-content digest. [`crate::FixLifecycle`] derives an
//! instrument UUID from market, classification, ISIN (else symbol) and currency,
//! and aligns updatedat to an epoch grid. These FIX recipes reuse the shared
//! algorithms; they do not define another hash engine. A raw
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
