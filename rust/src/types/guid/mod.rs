//! Globally unique identifier datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod scalars;

pub(crate) use dtypes::{GUID_EXTENSION_NAME, guid_bytes, guid_parse, guid_text};
pub use fields::*;
pub use scalars::{Guid, GuidScalar};
