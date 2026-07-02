//! # yggdryl-schema
//!
//! The Arrow-centralized schema layer of yggdryl: a typed data-type system and
//! the generic [`Field`] that names one.
//!
//! Every concrete type implements the base [`DataType`] trait — Arrow interop
//! via [`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow)
//! (total and reversible for the supported subset) and byte round-trips via
//! [`to_bytes`](DataType::to_bytes) / [`from_bytes`](DataType::from_bytes) —
//! plus the category subtraits that apply to it:
//!
//! - [`PrimitiveType`] — fixed-width types with a native Rust value type
//!   ([`Int32`], [`Float64`], [`Decimal128`], …);
//! - [`LogicalType`] — types carrying semantics over a physical anchor
//!   ([`Date32`] over [`Int32`], [`Timestamp`] over [`Int64`], …);
//! - [`NestedType`] — types containing child fields ([`List`], [`LargeList`]).
//!
//! Types are grouped one module per category (`integer`, `float`, `decimal`,
//! `string`, `binary`, `temporal`, `list`), one file per type, and re-exported
//! flat at the crate root. The trait is deliberately not object safe; the
//! object-safe erasure arrives with the `Datum` layer above this crate.
//!
//! ```
//! use yggdryl_schema::{DataType, Field, Int32};
//!
//! let field = Field::from_parts("id", Int32, false, Default::default());
//! let arrow = field.to_arrow();
//! assert_eq!(Field::from_arrow(&arrow), Ok(field));
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays free of the dependency by default and pays no
/// runtime cost). Reached from submodules via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod bytes;
mod datatype;
mod field;

pub use datatype::{
    Binary, Boolean, DataType, DataTypeError, Date32, Date64, Decimal128, Decimal256, Duration,
    FixedSizeBinary, Float32, Float64, Int16, Int32, Int64, Int8, LargeBinary, LargeList,
    LargeUtf8, List, LogicalType, NestedType, PrimitiveType, Time32, Time64, TimeUnit, Timestamp,
    UInt16, UInt32, UInt64, UInt8, Utf8,
};
pub use field::{Field, FieldError, FieldRef};
