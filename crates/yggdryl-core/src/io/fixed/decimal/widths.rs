//! The four **decimal widths** — the `Dec32` / `Dec64` / `Dec128` / `Dec256` [`DecimalBacking`]
//! markers and their `D*` value + `D*Type` / `D*Field` / `D*Scalar` / `D*Serie` aliases over the
//! generic decimal types. Each marker pins its coefficient integer, max precision, and Arrow
//! decimal type; `d256`'s coefficient is Arrow's [`i256`](arrow_buffer::i256), kept an
//! implementation detail (the public surface speaks in [`D256`] / `i128` / bytes / strings).

use super::{Decimal, DecimalBacking, DecimalField, DecimalScalar, DecimalSerie, DecimalType};
use crate::io::DataTypeId;

// ---- d32 — i32 coefficient (Arrow Decimal32, max precision 9) ------------------------
/// The `d32` decimal width marker — an `i32` coefficient (Arrow `Decimal32`, max precision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dec32;

impl DecimalBacking for Dec32 {
    type Coeff = i32;
    const NAME: &'static str = "d32";
    const WIDTH: usize = 4;
    const TYPE_ID: DataTypeId = DataTypeId::D32;
    const MAX_PRECISION: u8 = 9;
    #[cfg(feature = "arrow")]
    type Arrow = arrow_array::types::Decimal32Type;
}

/// A `d32` scaled-decimal value.
pub type D32 = Decimal<Dec32>;
/// The `d32` columnar descriptor (`precision`, `scale`).
pub type D32Type = DecimalType<Dec32>;
/// A named, nullable `d32` column descriptor.
pub type D32Field = DecimalField<Dec32>;
/// One nullable `d32` value carried with its column `(precision, scale)`.
pub type D32Scalar = DecimalScalar<Dec32>;
/// A nullable column of `d32` values.
pub type D32Serie = DecimalSerie<Dec32>;

// ---- d64 — i64 coefficient (Arrow Decimal64, max precision 18) -----------------------
/// The `d64` decimal width marker — an `i64` coefficient (Arrow `Decimal64`, max precision 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dec64;

impl DecimalBacking for Dec64 {
    type Coeff = i64;
    const NAME: &'static str = "d64";
    const WIDTH: usize = 8;
    const TYPE_ID: DataTypeId = DataTypeId::D64;
    const MAX_PRECISION: u8 = 18;
    #[cfg(feature = "arrow")]
    type Arrow = arrow_array::types::Decimal64Type;
}

/// A `d64` scaled-decimal value.
pub type D64 = Decimal<Dec64>;
/// The `d64` columnar descriptor (`precision`, `scale`).
pub type D64Type = DecimalType<Dec64>;
/// A named, nullable `d64` column descriptor.
pub type D64Field = DecimalField<Dec64>;
/// One nullable `d64` value carried with its column `(precision, scale)`.
pub type D64Scalar = DecimalScalar<Dec64>;
/// A nullable column of `d64` values.
pub type D64Serie = DecimalSerie<Dec64>;

// ---- d128 — i128 coefficient (Arrow Decimal128, max precision 38) --------------------
/// The `d128` decimal width marker — an `i128` coefficient (Arrow `Decimal128`, max precision 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dec128;

impl DecimalBacking for Dec128 {
    type Coeff = i128;
    const NAME: &'static str = "d128";
    const WIDTH: usize = 16;
    const TYPE_ID: DataTypeId = DataTypeId::D128;
    const MAX_PRECISION: u8 = 38;
    #[cfg(feature = "arrow")]
    type Arrow = arrow_array::types::Decimal128Type;
}

/// A `d128` scaled-decimal value.
pub type D128 = Decimal<Dec128>;
/// The `d128` columnar descriptor (`precision`, `scale`).
pub type D128Type = DecimalType<Dec128>;
/// A named, nullable `d128` column descriptor.
pub type D128Field = DecimalField<Dec128>;
/// One nullable `d128` value carried with its column `(precision, scale)`.
pub type D128Scalar = DecimalScalar<Dec128>;
/// A nullable column of `d128` values.
pub type D128Serie = DecimalSerie<Dec128>;

// ---- d256 — i256 coefficient (Arrow Decimal256, max precision 76) --------------------
/// The `d256` decimal width marker — a 256-bit coefficient (Arrow `Decimal256`, max precision 76).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dec256;

impl DecimalBacking for Dec256 {
    type Coeff = arrow_buffer::i256;
    const NAME: &'static str = "d256";
    const WIDTH: usize = 32;
    const TYPE_ID: DataTypeId = DataTypeId::D256;
    const MAX_PRECISION: u8 = 76;
    #[cfg(feature = "arrow")]
    type Arrow = arrow_array::types::Decimal256Type;
}

/// A `d256` scaled-decimal value.
pub type D256 = Decimal<Dec256>;
/// The `d256` columnar descriptor (`precision`, `scale`).
pub type D256Type = DecimalType<Dec256>;
/// A named, nullable `d256` column descriptor.
pub type D256Field = DecimalField<Dec256>;
/// One nullable `d256` value carried with its column `(precision, scale)`.
pub type D256Scalar = DecimalScalar<Dec256>;
/// A nullable column of `d256` values.
pub type D256Serie = DecimalSerie<Dec256>;
