//! Ordered software/protocol versions as one generic scalar value.
//!
//! A version is a four-byte numeric value: an eight-bit major, an eight-bit
//! minor, and a sixteen-bit patch. Missing minor and patch components are zero;
//! canonical text omits trailing zero components. A compact FIX `SP` suffix
//! supplies the numeric patch, case-insensitively: `5.0sp250` becomes `5.0.250`.
//! Qualifiers are not stored. Parsing and numeric comparison allocate nothing, and cloning
//! copies the four-byte value. This is neither an ASCII-width
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
