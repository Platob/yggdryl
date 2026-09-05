//! Binary and fixed-width byte datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub use scalars::{
    Binary, BinaryScalar, BinaryView, BinaryViewScalar, Bytes, BytesValue, FixedSizeBinary,
    FixedSizeBinaryScalar, LargeBinary, LargeBinaryScalar,
};
