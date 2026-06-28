//! The **primitive** concrete series — the fixed-width Arrow scalars plus strings and
//! binary. [`PrimitiveSerie<A>`] covers every numeric / temporal / decimal type;
//! [`BooleanSerie`], [`VarcharSerie<O>`] and [`BinarySerie<O>`] cover the rest. The
//! named aliases below pin the common widths so callers write `Int32Serie` rather than
//! `PrimitiveSerie<Int32Type>`.

mod binary;
mod boolean;
mod null;
mod numeric;
mod varchar;

pub use binary::BinarySerie;
pub use boolean::BooleanSerie;
pub use null::NullSerie;
pub use numeric::PrimitiveSerie;
pub use varchar::VarcharSerie;

use arrow_array::types::{
    Date32Type, Date64Type, Decimal128Type, Decimal256Type, Float16Type, Float32Type, Float64Type,
    Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};

/// An `int8` column.
pub type Int8Serie = PrimitiveSerie<Int8Type>;
/// An `int16` column.
pub type Int16Serie = PrimitiveSerie<Int16Type>;
/// An `int32` column.
pub type Int32Serie = PrimitiveSerie<Int32Type>;
/// An `int64` column.
pub type Int64Serie = PrimitiveSerie<Int64Type>;
/// A `uint8` column.
pub type UInt8Serie = PrimitiveSerie<UInt8Type>;
/// A `uint16` column.
pub type UInt16Serie = PrimitiveSerie<UInt16Type>;
/// A `uint32` column.
pub type UInt32Serie = PrimitiveSerie<UInt32Type>;
/// A `uint64` column.
pub type UInt64Serie = PrimitiveSerie<UInt64Type>;

/// A `float16` column.
pub type Float16Serie = PrimitiveSerie<Float16Type>;
/// A `float32` column.
pub type Float32Serie = PrimitiveSerie<Float32Type>;
/// A `float64` column.
pub type Float64Serie = PrimitiveSerie<Float64Type>;

/// A 128-bit decimal column.
pub type Decimal128Serie = PrimitiveSerie<Decimal128Type>;
/// A 256-bit decimal column.
pub type Decimal256Serie = PrimitiveSerie<Decimal256Type>;

/// A day-resolution date column (`int32` storage).
pub type Date32Serie = PrimitiveSerie<Date32Type>;
/// A millisecond-resolution date column (`int64` storage).
pub type Date64Serie = PrimitiveSerie<Date64Type>;

// Timestamps, times and durations are not aliased here — every unit unifies into the
// [`DatetimeSerie`](crate::DatetimeSerie) / [`TimeSerie`](crate::TimeSerie) /
// [`DurationSerie`](crate::DurationSerie).
