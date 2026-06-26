//! # yggdryl-saga
//!
//! The columnar, **Arrow-convertible** dataframe core of the **yggdryl** project:
//! a lazy, zero-copy engine aimed at sorted timeseries at large scale. This first
//! layer is the **schema vocabulary** — the value types that describe the shape of
//! a column — built to mirror Apache Arrow exactly so a schema crosses the
//! `arrow-schema` boundary at zero cost.
//!
//! - [`DataType`] is the logical type of a column, split (as Arrow's own types
//!   are) into three families:
//!   - [`PrimitiveType`] — the flat, child-less scalars (`Int64`, `Float64`,
//!     `Utf8`, `FixedSizeBinary`, …);
//!   - [`LogicalType`] — semantic types over a physical layout (`Timestamp`,
//!     `Date32`, `Decimal128`, `Duration`, …);
//!   - [`NestedType`] — types that carry child [`Field`]s (`List`, `Struct`,
//!     `Map`, `Union`, `Dictionary`, …).
//! - [`Field`] is a named, nullable [`DataType`] with metadata — the column
//!   header; [`Schema`] is an ordered list of them.
//! - [`DataType::Any`] is the **dynamic** type for untyped literals, and
//!   [`can_cast_to`](DataType::can_cast_to) the casting rule that types them.
//!
//! On top of that vocabulary sit the **base traits** that every future frame and
//! column backing will satisfy, so eager and lazy implementations share one
//! surface:
//!
//! - [`Column`] — a single named, typed column, materialized or lazy;
//! - [`Frame`] — a tabular frame: `select` / `filter` / column access over a
//!   common [`Schema`], whether the rows are in memory or still a plan.
//!
//! The first concrete backing (the on-by-default `dataframe` feature) is the eager,
//! Arrow-`RecordBatch`-backed [`DataFrame`] / [`ArrayColumn`]: projection and
//! row-slicing are zero-copy, `filter` types the predicate's literals against the
//! schema before evaluating it, and [`group_by`](DataFrame::group_by) /
//! [`resample`](DataFrame::resample) reduce rows with [`Agg`]s — taking a
//! single-pass, hash-free path over sorted timeseries (`resample` buckets, and a
//! sorted `group_by` key, are contiguous row ranges).
//!
//! …and the **filtering layer** they consume:
//!
//! - [`Scalar`] — a typed literal; [`cast`](Scalar::cast) types an untyped
//!   ([`Any`](DataType::Any)) or string value (e.g. an ISO date → a `timestamp`);
//! - [`Expression`] / [`Col`] / [`Lit`] — expression nodes that resolve a type
//!   against a [`Schema`];
//! - [`Predicate`] — a boolean filter whose [`optimize`](Predicate::optimize)
//!   casts each literal to its column's type, so [`Frame::filter`] can push it
//!   down into typed storage.
//!
//! Every value type pairs a canonical-string [`from_str`](DataType::from_str) /
//! [`to_str`](DataType::to_str) round-trip with, under the on-by-default `arrow`
//! feature, infallible conversions to and from the matching `arrow-schema` type
//! ([`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow)). Our
//! [`DataType`] is a total partition of `arrow_schema::DataType`, so the
//! conversion is a lossless bijection in both directions.
//!
//! ```
//! use yggdryl_saga::{DataType, Field, PrimitiveType};
//!
//! let dt = DataType::from_str("list<item: int64>").unwrap();
//! assert!(dt.is_nested());
//! assert_eq!(dt.to_str(), "list<item: int64>");
//!
//! let f = Field::new("price", DataType::from(PrimitiveType::Float64), true);
//! assert_eq!(f.to_str(), "price: float64");
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

mod parse;

mod datatype;
mod field;
mod schema;

mod cast;
mod expression;
mod predicate;
mod scalar;

mod agg;
mod period;

mod column;
mod frame;

#[cfg(feature = "dataframe")]
mod array_column;
#[cfg(feature = "dataframe")]
mod dataframe;
#[cfg(feature = "dataframe")]
mod groupby;

pub use datatype::{
    DataType, DataTypeError, IntervalUnit, LogicalType, NestedType, PrimitiveType, TimeUnit,
    UnionMode,
};
pub use field::{Field, FieldError};
pub use schema::{Schema, SchemaError};

pub use cast::CastError;
pub use expression::{col, lit, Col, Expression, ExpressionError, Lit};
pub use predicate::{CompareOp, Predicate};
pub use scalar::Scalar;

pub use agg::{Agg, AggFunc};
pub use period::{Period, PeriodError};

pub use column::{Column, ColumnError};
pub use frame::{Frame, FrameError, FrameHandle};

#[cfg(feature = "dataframe")]
pub use array_column::ArrayColumn;
#[cfg(feature = "dataframe")]
pub use dataframe::DataFrame;
#[cfg(feature = "dataframe")]
pub use groupby::{GroupBy, Resample};
