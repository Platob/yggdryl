//! Fixed-width decimals — an integer **mantissa** scaled by a power of ten
//! (`value = mantissa × 10^(−scale)`), matching Apache Arrow's decimal widths.
//!
//! The base [`Decimal`] trait is the FFI-opaque surface over the four widths
//! [`Decimal32`] / [`Decimal64`] / [`Decimal128`] / [`Decimal256`]. Each is byte-based
//! (mantissa little-endian bytes + a scale byte), has value semantics (equal iff its
//! `serialize_bytes` are equal), converts to an [`f64`] / integer, rescales, and widens or
//! narrows between the widths (`to_decimal256` up, `Decimal256::try_to_decimal128` down,
//! and `From` for the exact native widenings).

// The base trait `Decimal` lives in `decimal.rs`, named for the type it holds (rule 1).
#[allow(clippy::module_inception)]
mod decimal;
mod decimal128;
mod decimal256;
mod decimal32;
mod decimal64;
mod decimal_error;
mod primitive;

pub use decimal::Decimal;
pub use decimal128::Decimal128;
pub use decimal256::Decimal256;
pub use decimal32::Decimal32;
pub use decimal64::Decimal64;
pub use decimal_error::DecimalError;
