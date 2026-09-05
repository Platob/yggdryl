//! Signed and unsigned integer datatypes.

mod dtypes;
mod fields;
pub(crate) mod scalars;

pub use fields::*;
pub use scalars::{
    Int8, Int16, Int32, Int64, Int128, Integer, IntegerValue, UInt8, UInt16, UInt32, UInt64,
    UInt128,
};
pub(crate) use scalars::{
    canonical_signed, canonical_unsigned, validate_integer_tuple, validate_signed,
    validate_unsigned,
};
