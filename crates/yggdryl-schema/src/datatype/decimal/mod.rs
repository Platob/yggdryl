//! Fixed-point decimal data types.

mod decimal128;
mod decimal256;
pub(crate) mod decimal_type;

pub use decimal128::Decimal128Type;
pub use decimal256::Decimal256Type;
pub use decimal_type::DecimalType;
