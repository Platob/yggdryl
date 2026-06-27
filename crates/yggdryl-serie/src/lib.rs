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
//! - [`Serie`] — the object-safe **base** trait every column implements: convenience
//!   field reflections ([`name`](Serie::name) / [`dtype`](Serie::dtype) /
//!   [`get_metadata`](Serie::get_metadata)); the backing [`field`](Serie::field) /
//!   [`array`](Serie::array); the [`len`](Serie::len) / [`num_rows`](Serie::num_rows) /
//!   [`null_count`](Serie::null_count) bookkeeping; type-erased value access by index
//!   ([`value_at`](Serie::value_at) → [`Scalar`]) and by range
//!   ([`slice`](Serie::slice) / [`slice_range`](Serie::slice_range), zero-copy); the
//!   [`parent`](Serie::parent) graph link; [`materialize`](Serie::materialize); and
//!   downcasting via [`as_any`](Serie::as_any).
//! - [`TypedSerie<T>`] — typed value access (`get` / `value` / `iter`) over the native
//!   value type `T` of a concrete column.
//! - The **primitive** concrete series — [`PrimitiveSerie<A>`] (Arrow numeric / date /
//!   time / duration / interval types), [`BooleanSerie`], [`VarcharSerie<O>`] and
//!   [`BinarySerie<O>`].
//! - The **temporal** series — [`DatetimeSerie`] / [`TimeSerie`] / [`DurationSerie`]
//!   (unified columns over any unit, presenting core `DateTime` / `Time` / `Duration`)
//!   and the [`TemporalSerie`] trait (`datetime_at` / `date_at` / `time_at`).
//! - The **nested** series — [`StructSerie`], [`ListSerie<O>`] and [`MapSerie`] (child
//!   columns built recursively) and the [`NestedSerie`] trait.
//! - The **lazy** (computed) series — [`RangeSerie`], [`DateRangeSerie`],
//!   [`DateTimeRangeSerie`] and [`TimeRangeSerie`] — store a compact description and
//!   produce values on demand until materialised (the temporal ranges are
//!   [`TemporalSerie`]s).
//! - [`IndexSerie`] — a row index, defaulting to a lazy `uint64` [`RangeSerie`].
//! - [`EnumSerie`] — a categorical view holding the unique values mapped to their code
//!   and first row index.
//! - [`SliceSerie`] / [`child`] — zero-copy child views that record their
//!   [`parent`](Serie::parent), forming a slice graph.
//! - [`Serie::display`] with [`DisplayOptions`] renders a column to a readable string.
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

mod build;
mod display;
mod enum_serie;
mod error;
mod index;
mod lazy;
mod nested;
mod path;
mod primitive;
mod scalar;
mod serie;
mod slice;
mod temporal;

#[cfg(test)]
mod tests;

pub use display::DisplayOptions;
pub use enum_serie::EnumSerie;
pub use error::{SerieError, SerieResult};
pub use index::IndexSerie;
pub use lazy::{DateRangeSerie, DateTimeRangeSerie, RangeSerie, TimeRangeSerie};
pub use nested::{ListSerie, MapSerie, NestedSerie, StructSerie};
pub use primitive::{
    BinarySerie,
    BooleanSerie,
    // Concrete date / decimal aliases.
    Date32Serie,
    Date64Serie,
    Decimal128Serie,
    Decimal256Serie,
    // Concrete numeric aliases.
    Float16Serie,
    Float32Serie,
    Float64Serie,
    Int16Serie,
    Int32Serie,
    Int64Serie,
    Int8Serie,
    PrimitiveSerie,
    UInt16Serie,
    UInt32Serie,
    UInt64Serie,
    UInt8Serie,
    VarcharSerie,
};
pub use scalar::Scalar;
pub use serie::{from_array, from_arrow, Serie, SerieRef, TypedSerie};
pub use slice::{child, child_range, SliceSerie};
pub use temporal::{DatetimeSerie, DurationSerie, TemporalSerie, TimeSerie};

// Re-export the Arrow array crate so dependents build arrays without pinning the
// exact `arrow-array` version themselves, and the shared vocabulary they need.
pub use arrow_array;
pub use yggdryl_schema::{DataType, Field, TypeCategory};
