//! Exact decimal datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod parser;
pub(crate) mod scalars;

pub use dtypes::DecimalType;
pub(crate) use dtypes::validate_decimal;
pub use fields::*;
pub use scalars::{Decimal, Decimal32, Decimal64, Decimal128, Decimal256, DecimalValue};
pub(crate) use scalars::{validate_decimal_value, validate_decimal256_value};
