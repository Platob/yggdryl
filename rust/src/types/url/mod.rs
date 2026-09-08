//! Validated locations as one generic scalar value.
//!
//! A URL is the crate's [`Url`](crate::Url) - the same parsed value a handle
//! addresses itself by - carried as a column. Parsing canonicalizes: a scheme
//! and percent-encoding fold to their canonical case, a bare platform path
//! becomes a `file:` URL, and re-parsing the canonical text answers the same
//! value. Arrow stores that text as Utf8; its extension name preserves the
//! datatype on a field round trip, so a column that went out as a URL comes
//! back as one rather than as prose that happens to look like a location.
//!
//! Ordering is the canonical text's, which is Arrow's own string ordering:
//! there is no numeric component to sort by, as there is for
//! [`Version`](crate::Version).
//!
//! The value is held behind one shared pointer. A parsed URL is sixteen times
//! the width of the scalar root, and a column of them is cloned once per row,
//! so a row clone moves a reference count rather than a URI.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod fields;
mod value;

pub use fields::{UrlField, UrlScalar, UrlType};

/// The Arrow extension name preserving [`crate::DataType::Url`] over its Utf8
/// storage.
pub(crate) const URL_EXTENSION_NAME: &str = "yggdryl.url";
