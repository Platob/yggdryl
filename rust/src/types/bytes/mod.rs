//! Binary and fixed-width byte datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub use scalars::{
    BinaryScalar, BinaryViewScalar, BytesValue, FixedSizeBinaryScalar, LargeBinaryScalar,
};
