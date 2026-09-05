//! Signed and unsigned integer datatypes.

mod dtypes;
mod fields;
pub(crate) mod scalars;

pub use fields::*;
pub use scalars::IntegerValue;
pub(crate) use scalars::{
    canonical_signed, canonical_unsigned, validate_integer_tuple, validate_signed,
    validate_unsigned,
};
