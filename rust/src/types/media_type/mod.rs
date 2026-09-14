//! The media type as a datatype: a column of canonical
//! `type/subtype; charset=...` names with their content codings.
//!
//! The value itself lives beside [`crate::MediaType`] - a media type is what a
//! record layer is read and written under, and there is one of those. This is
//! only what makes that value a datatype, a field and a scalar.
//!
//! Storage is Arrow's `Utf8`: the base, the charset and the codings are one
//! canonical rendering, so a cell holds what the value spells and the
//! extension name keeps it a media type across a round trip. The value is
//! wider than the scalar enum - a base, a charset and a shared coding list -
//! so a scalar holds it behind one shared pointer, exactly as a URL is held.
//!
//! ```
//! use yggdryl::{DataType, MediaType, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(DataType::from_str("mediatype")?, DataType::MediaType);
//! let json = MediaType::from_str("application/json")?;
//! assert_eq!(DataType::MediaType.scalar("application/json")?, Scalar::from(json));
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod fields;
mod value;

pub use fields::{MediaTypeField, MediaTypeType};

/// The Arrow extension name preserving [`crate::DataType::MediaType`] over its
/// Utf8 storage.
pub(crate) const MEDIATYPE_EXTENSION_NAME: &str = "yggdryl.mediatype";
