//! Globally unique identifier datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod scalars;

pub(crate) use dtypes::{UUID_EXTENSION_NAME, uuid_bytes, uuid_parse, uuid_text};
pub use fields::*;
pub use scalars::{Uuid, UuidScalar};
