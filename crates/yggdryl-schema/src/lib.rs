//! # yggdryl-schema
//!
//! The Arrow-centralized schema layer of yggdryl: a typed data-type system and
//! the generic [`Field`] that names one.
//!
//! Every concrete type implements the base [`DataType`] trait — Arrow interop
//! via [`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow)
//! (total and reversible for the supported subset), byte round-trips via
//! [`to_bytes`](DataType::to_bytes) / [`from_bytes`](DataType::from_bytes),
//! and the stable integer identifier of its constructor via
//! [`type_id`](DataType::type_id) ([`DataTypeId`]) — plus the category
//! subtraits that apply to it:
//!
//! - [`PrimitiveType`] — fixed-width types with a native Rust value type
//!   ([`Int32`], [`Float64`], [`Decimal128`], …);
//! - [`LogicalType`] — types carrying semantics over a physical anchor
//!   ([`Date32`] over [`Int32`], [`Timestamp`] over [`Int64`], …);
//! - [`NestedType`] — types containing child fields ([`List`], [`Struct`],
//!   [`Map`], …).
//!
//! Types are grouped one module per category (`integer`, `float`, `decimal`,
//! `string`, `binary`, `temporal`, `list`, …), one file per type, and
//! re-exported flat at the crate root. Heterogeneous collections hold the
//! erased [`AnyDataType`], which implements [`DataType`] by delegating to the
//! wrapped concrete type.
//!
//! Fields follow the same shape: the abstract [`Field`] base defines the
//! surface (a name, a data type, nullability, metadata, plus the provided
//! Arrow and byte conversions) and the generic [`TypedField`] is the
//! implementation covering every data type.
//!
//! ```
//! use yggdryl_schema::{DataType, Field, Int32, TypedField};
//!
//! let field = TypedField::from_parts("id", Int32, false, Default::default());
//! let arrow = field.to_arrow();
//! assert_eq!(TypedField::from_arrow(&arrow), Ok(field));
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
pub mod metadata;

pub use datatype::{
    AnyDataType, AnyTime32Unit, AnyTime64Unit, AnyTimeUnit, Binary, Boolean, DataType,
    DataTypeError, DataTypeId, Date, Date32, Date64, Day, Decimal128, Decimal256, Duration,
    FixedSizeBinary, Float32, Float64, Hour, Int16, Int32, Int64, Int8, LargeBinary, LargeList,
    LargeUtf8, List, LogicalType, Map, Microsecond, Millisecond, Minute, Month, Nanosecond,
    NestedType, PrimitiveType, Quarter, Second, Struct, Time, Time32, Time32Unit, Time64,
    Time64Unit, TimeUnit, TimeUnitId, Timestamp, TypedDuration, TypedTimestamp, UInt16, UInt32,
    UInt64, UInt8, Utf8, Week, Year,
};
pub use field::{Field, FieldError, TypedField, TypedFieldRef};
