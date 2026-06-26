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
//!
//! On top of that vocabulary sit the **base traits** that every future frame and
//! column backing will satisfy, so eager and lazy implementations share one
//! surface:
//!
//! - [`Column`] — a single named, typed column, materialized or lazy;
//! - [`Frame`] — a tabular frame: `select` / `filter` / column access over a
//!   common [`Schema`], whether the rows are in memory or still a plan.
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

mod column;
mod frame;

pub use datatype::{
    DataType, DataTypeError, IntervalUnit, LogicalType, NestedType, PrimitiveType, TimeUnit,
    UnionMode,
};
pub use field::{Field, FieldError};
pub use schema::{Schema, SchemaError};

pub use column::{Column, ColumnError};
pub use frame::{Frame, FrameError};
