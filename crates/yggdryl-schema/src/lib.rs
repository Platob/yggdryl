//! # yggdryl-schema
//!
//! A compact, **Arrow-compatible** schema layer for yggdryl, built to back a future
//! dataframe. Everything is centred on just two types:
//!
//! - [`DataType`] — the logical type of a value, in three [categories](TypeCategory):
//!   **primitive** ([`Int32`](DataType::Int32), [`Float64`](DataType::Float64),
//!   [`Varchar`](DataType::Varchar), …), **logical** ([`Timestamp`](DataType::Timestamp),
//!   [`Decimal128`](DataType::Decimal128), [`Dictionary`](DataType::Dictionary), …) and
//!   **nested** ([`List`](DataType::List), [`Struct`](DataType::Struct), …), plus the
//!   [`Any`](DataType::Any) wildcard. The fixed-width numerics are concrete variants,
//!   each a struct generic over its native Rust storage type (`Int64<i64>`, …);
//!   the variable-width attributes stay uniform accessors:
//!   [`byte_size`](DataType::byte_size) / [`is_large`](DataType::is_large) /
//!   [`is_view`](DataType::is_view), and strings are a single [`Varchar`](DataType::Varchar)
//!   with a [`Charset`].
//! - [`Field`] — a named, nullable [`DataType`] with metadata, an optional
//!   [`parent`](Field::parent) for graph traversal and child accessors. A `Field`
//!   whose type is a [`Struct`](DataType::Struct) **is** a schema (convertible to an
//!   Arrow `Schema`).
//!
//! On top sit the operations a batch / column store needs: fast type checks
//! ([`DataType::is_numeric`], …), a conversion lattice ([`DataType::can_cast_to`] /
//! [`common_type`](DataType::common_type)) and a [`merge`](DataType::merge) strategy
//! ([`MergeStrategy`]). Temporal types reuse the core [`TimeUnit`](yggdryl_core::TimeUnit)
//! and [`Timezone`](yggdryl_core::Timezone).
//!
//! Every type converts smoothly to/from a string, a `BTreeMap`,
//! JSON and bytes, is `serde`-serializable (under `serde`) and [`Hash`]; with the
//! `arrow` feature it converts losslessly to/from `arrow-schema`.
//!
//! ```
//! use yggdryl_schema::{DataType, Field};
//!
//! let schema = DataType::struct_(vec![
//!     Field::new("id", DataType::int(64, true), false),
//!     Field::new("name", DataType::varchar(), true),
//! ]);
//! assert_eq!(DataType::from_str("struct[id: int64 not null, name: utf8]").unwrap(), schema);
//! assert!(DataType::int(32, true).can_cast_to(&DataType::int(64, true)));
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

mod datatype;
mod field;

#[cfg(feature = "arrow")]
mod arrow;

pub use datatype::fixed::{
    f16, i256, Decimal128, Decimal256, Decimal32, Decimal64, FixedInfo, FixedKind, Float16,
    Float32, Float64, Int16, Int32, Int64, Int8, Native, UInt16, UInt32, UInt64, UInt8,
};
pub use datatype::{
    DataType, IntervalUnit, MergeStrategy, Numeric, SchemaError, TypeCategory, UnionMode,
};
pub use field::Field;

// Re-export the shared vocabulary the schema types build on, so dependents resolve
// `yggdryl_schema::{Charset, TimeUnit, Timezone}` without a separate core import.
pub use yggdryl_core::{Charset, TimeUnit, Timezone};
