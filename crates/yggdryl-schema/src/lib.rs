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
//!   ([`Int32Type`], [`Float64Type`], [`Decimal128Type`], …);
//! - [`LogicalType`] — types carrying semantics over a physical anchor
//!   ([`Date32Type`] over [`Int32Type`], [`Timestamp`] over [`Int64Type`], …);
//! - [`NestedType`] — types containing child fields ([`ListType`], [`StructType`],
//!   [`MapType`], …).
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
//! use yggdryl_schema::{DataType, Field, Int32Type, TypedField};
//!
//! let field = TypedField::from_parts("id", Int32Type, false, Default::default());
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
    AnyDataType, AnyTime32Unit, AnyTime64Unit, AnyTimeUnit, BinaryType, BooleanType, DataType,
    DataTypeError, DataTypeId, Date, Date32Type, Date64Type, Day, Decimal128Type, Decimal256Type,
    DecimalType, Duration, DurationType, FixedSizeBinaryType, Float32Type, Float64Type, FloatType,
    Hour, Int16Type, Int32Type, Int64Type, Int8Type, IntegerType, LargeBinaryType, LargeListType,
    LargeUtf8Type, ListType, LogicalType, MapType, Microsecond, Millisecond, Minute, Month,
    Nanosecond, NestedType, NumericType, PrimitiveType, Quarter, Second, StructType, TemporalType,
    Time, Time32Type, Time32Unit, Time64Type, Time64Unit, TimeUnit, TimeUnitId, Timestamp,
    TimestampType, UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type, Week, Year,
};
pub use field::{
    AnyField, BinaryField, BooleanField, Date32Field, Date64Field, Decimal128Field,
    Decimal256Field, DurationField, Field, FieldError, FixedSizeBinaryField, Float32Field,
    Float64Field, Int16Field, Int32Field, Int64Field, Int8Field, LargeBinaryField, LargeListField,
    LargeUtf8Field, ListField, MapField, StructField, Time32Field, Time64Field, TimestampField,
    TypedField, TypedFieldRef, UInt16Field, UInt32Field, UInt64Field, UInt8Field, Utf8Field,
};
