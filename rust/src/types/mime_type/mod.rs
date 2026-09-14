//! The MIME type as a datatype: a column of canonical `type/subtype` names.
//!
//! The value itself lives beside [`crate::MimeType`] - a MIME type is what a
//! handle says its bytes are, and there is one of those, not one for media
//! routing and another for columns. This is only what makes that value a
//! datatype, a field and a scalar.
//!
//! Storage is Arrow's `Utf8`, which is what the canonical name is; the
//! extension name is what keeps a MIME column a MIME column across a round
//! trip. Case and parameters canonicalize on the way in, exactly as
//! [`crate::MimeType::from_str`] canonicalizes them everywhere else.
//!
//! ```
//! use yggdryl::{DataType, MimeType, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(DataType::from_str("mimetype")?, DataType::MimeType);
//! assert_eq!(
//!     DataType::MimeType.scalar("APPLICATION/JSON")?,
//!     Scalar::MimeType(MimeType::JSON),
//! );
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod fields;
mod value;

pub use fields::{MimeTypeField, MimeTypeType};

/// The Arrow extension name preserving [`crate::DataType::MimeType`] over its
/// Utf8 storage.
pub(crate) const MIMETYPE_EXTENSION_NAME: &str = "yggdryl.mimetype";
