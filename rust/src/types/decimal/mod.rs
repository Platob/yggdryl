//! Exact decimal datatypes.

mod dtypes;
mod fields;
mod parser;
pub(crate) mod scalars;

pub(crate) use dtypes::validate_decimal;
pub use fields::*;
pub(crate) use scalars::{validate_decimal_value, validate_decimal256_value};
