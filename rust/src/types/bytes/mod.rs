//! Binary and fixed-width byte datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod scalars;

pub use dtypes::BytesType;
pub use fields::*;
pub(crate) use scalars::bytes_from_value;
pub use scalars::{Binary, BinaryView, Bytes, BytesValue, FixedSizeBinary, LargeBinary};
