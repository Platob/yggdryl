//! Exact decimal datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod parser;
pub(crate) mod scalars;

pub(crate) use dtypes::validate_decimal;
pub use fields::*;
pub use scalars::DecimalValue;
pub(crate) use scalars::{validate_decimal_value, validate_decimal256_value};
