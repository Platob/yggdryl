//! # yggdryl-serie
//!
//! Arrow-backed **columnar series** for yggdryl — the layer between the
//! [`yggdryl-schema`](yggdryl_schema) type system and a future dataframe. A
//! [`Serie`] is a single named, typed column: a [`Field`](yggdryl_schema::Field)
//! (its name + [`DataType`](yggdryl_schema::DataType) + nullability + metadata)
//! paired with an Apache Arrow array holding the values.
//!
//! The design mirrors the schema crate's three [categories](yggdryl_schema::TypeCategory):
//!
//! - [`Serie`] — the object-safe **base** trait every column implements: accessors to
//!   the [`field`](Serie::field) and the backing [`array`](Serie::array), the length /
//!   null bookkeeping, [`slice`](Serie::slice) and downcasting via
//!   [`as_any`](Serie::as_any).
//! - [`TypedSerie<T>`] — typed value access (`get` / `value` / `iter`) over the native
//!   value type `T` of a concrete column.
//! - The **primitive** concrete series — [`PrimitiveSerie<A>`] (every Arrow numeric and
//!   temporal type), [`BooleanSerie`], [`VarcharSerie<O>`] and [`BinarySerie<O>`].
//!
//! [`from_arrow`] / [`from_array`] **redirect** an Arrow array to the right concrete
//! series, returning a boxed [`SerieRef`] — the basis for a column store, and in turn
//! a `Frame` / `LazyFrame` / `ParquetFrame`.
//!
//! ```
//! use yggdryl_serie::{from_array, TypedSerie};
//! use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
//! use std::sync::Arc;
//!
//! let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
//! let serie = from_array("id", array).unwrap();
//! assert_eq!(serie.len(), 3);
//! assert_eq!(serie.null_count(), 1);
//! assert_eq!(serie.name(), "id");
//!
//! // typed access through a downcast
//! let ints = serie.as_any().downcast_ref::<yggdryl_serie::Int32Serie>().unwrap();
//! assert_eq!(ints.get(0), Some(1));
//! assert_eq!(ints.get(1), None);
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Shared by every submodule via `pub(crate) use log_event`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod error;
mod primitive;
mod serie;

#[cfg(test)]
mod tests;

pub use error::{SerieError, SerieResult};
pub use primitive::{
    BinarySerie,
    BooleanSerie,
    // Concrete temporal / decimal aliases.
    Date32Serie,
    Date64Serie,
    Decimal128Serie,
    Decimal256Serie,
    DurationMicrosecondSerie,
    DurationMillisecondSerie,
    DurationNanosecondSerie,
    DurationSecondSerie,
    // Concrete numeric aliases.
    Float16Serie,
    Float32Serie,
    Float64Serie,
    Int16Serie,
    Int32Serie,
    Int64Serie,
    Int8Serie,
    PrimitiveSerie,
    TimestampMicrosecondSerie,
    TimestampMillisecondSerie,
    TimestampNanosecondSerie,
    TimestampSecondSerie,
    UInt16Serie,
    UInt32Serie,
    UInt64Serie,
    UInt8Serie,
    VarcharSerie,
};
pub use serie::{from_array, from_arrow, Serie, SerieRef, TypedSerie};

// Re-export the Arrow array crate so dependents build arrays without pinning the
// exact `arrow-array` version themselves, and the shared vocabulary they need.
pub use arrow_array;
pub use yggdryl_schema::{DataType, Field, TypeCategory};
