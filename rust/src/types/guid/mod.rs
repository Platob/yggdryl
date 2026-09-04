//! Globally unique identifier datatypes.

mod dtypes;
mod fields;
mod scalars;

pub(crate) use dtypes::{GUID_EXTENSION_NAME, guid_bytes, guid_parse, guid_text};
pub use fields::*;
pub use scalars::GuidScalar;
