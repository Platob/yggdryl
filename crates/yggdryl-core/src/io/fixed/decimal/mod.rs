//! `io::fixed::decimal` — the fixed-width **scaled-decimal** family: `d32`, `d64`, `d128`, `d256`
//! (Arrow `Decimal32`/`Decimal64`/`Decimal128`/`Decimal256`). A decimal is `coefficient × 10^-scale`
//! over a two's-complement coefficient integer — `i32`/`i64`/`i128` and Arrow's 256-bit
//! [`i256`](arrow_buffer::i256) — so it reports
//! [`DataTypeCategory::Decimal`](crate::io::DataTypeCategory::Decimal) and `dt.is_decimal()` drills
//! down without matching.
//!
//! The family has **two faces**, tied together by the shared [`DecimalBacking`] / [`DecimalCoeff`]
//! traits (one impl per width):
//!
//! - The self-describing **value type** [`Decimal<B>`] (`D32`/`D64`/`D128`/`D256`) — each value
//!   carries its own scale, with full checked arithmetic (`+`/`-`/`*`, [`checked_div`](Decimal::checked_div),
//!   …), ordering, `Display`/`FromStr`, a byte codec, and conversions to/from integers, floats, and
//!   the other widths. This is the "native Rust decimal".
//! - The **columnar** descriptors [`DecimalType<B>`] / [`DecimalField<B>`] / [`DecimalScalar<B>`] /
//!   [`DecimalSerie<B>`] — one `(precision, scale)` fixed per column (Arrow's model), storing raw
//!   coefficients and converting **zero-copy** to/from Arrow's decimal arrays (feature `arrow`).
//!
//! A [`DecimalScalar`] / [`DecimalSerie`] element *is* a [`Decimal`] value (its coefficient at the
//! column's scale), so the two faces compose: read a value out of a column, do arithmetic on it,
//! push it back.
//!
//! | width | value | coefficient | max precision | Arrow |
//! | --- | --- | --- | --- | --- |
//! | `d32`  | [`D32`]  | `i32`  | 9  | `Decimal32`  |
//! | `d64`  | [`D64`]  | `i64`  | 18 | `Decimal64`  |
//! | `d128` | [`D128`] | `i128` | 38 | `Decimal128` |
//! | `d256` | [`D256`] | `i256` | 76 | `Decimal256` |

mod backing;
mod dtype;
mod error;
mod field;
mod scalar;
mod serie;
mod value;

// The four concrete widths — each a marker + its `Decimal*`/`D*` aliases over the generics.
mod widths;

// The shared decimal traits.
pub use backing::{DecimalBacking, DecimalCoeff};

// The generic value + columnar types.
pub use dtype::DecimalType;
pub use error::DecimalError;
pub use field::DecimalField;
pub use scalar::DecimalScalar;
pub use serie::DecimalSerie;
pub use value::Decimal;

// The per-width markers and their aliases.
pub use widths::{
    D128Field, D128Scalar, D128Serie, D128Type, D256Field, D256Scalar, D256Serie, D256Type,
    D32Field, D32Scalar, D32Serie, D32Type, D64Field, D64Scalar, D64Serie, D64Type, Dec128, Dec256,
    Dec32, Dec64, D128, D256, D32, D64,
};
