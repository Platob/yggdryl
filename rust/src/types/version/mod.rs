//! Ordered software/protocol versions as one generic scalar value.
//!
//! A version is a sixteen-byte value: a required eight-bit major, an optional
//! eight-bit minor, and an optional fixed fourteen-byte patch/qualifier tail.
//! Parsing canonicalizes equivalent component and qualifier separators, while
//! ordering remains numeric (`SP2 < SP10`). Appended and dot-introduced FIX
//! service packs are post-releases; only a hyphen introduces a pre-release.
//! The fixed tail keeps every accepted value inline and bounds parse, compare,
//! clone, and render without a heap allocation. This is neither an ASCII-width
//! datatype nor a static coded vocabulary. Arrow stores the canonical text as
//! Utf8; its extension name preserves the datatype on a field round trip.
//! Arrow's own string ordering is consequently lexicographic—[`Version::cmp`]
//! is the ordering contract.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod fields;
mod value;

pub use fields::{VersionField, VersionScalar, VersionType};
pub use value::Version;

/// The Arrow extension name preserving [`crate::DataType::Version`] over
/// its Utf8 storage.
pub(crate) const VERSION_EXTENSION_NAME: &str = "yggdryl.version";
