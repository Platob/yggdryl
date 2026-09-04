//! Binary and fixed-width byte datatypes.

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub use scalars::{BinaryScalar, BinaryViewScalar, FixedSizeBinaryScalar, LargeBinaryScalar};
