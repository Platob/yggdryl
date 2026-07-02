//! # yggdryl-data
//!
//! The Apache Arrow-centralized data-model layer for yggdryl, built on top of
//! `yggdryl-core`. It defines the physical type system — data types, fields and
//! scalars — with zero-copy FFI and Arrow interop in mind.
//!
//! The type system comes in three layers of traits, each re-exported at the crate
//! root:
//!
//! - The **untyped base** [`RawDataType`], [`RawField`] and [`RawScalar`] — the
//!   FFI-facing descriptors.
//! - The **typed** [`DataType`], [`Field`] and [`Scalar`], parameterised by a native
//!   Rust type `T` (the [`DataType`] codec bridges a `T` to and from Arrow bytes).
//! - The **categories** [`Primitive`], [`Logical`] and [`Nested`] describing a type's
//!   shape.
//!
//! Concrete types live in per-family modules — the [`integer`] module holds every
//! signed and unsigned integer, the [`null`] module the storage-free null type, the
//! [`union`] module the union type, the [`optional`] module the logical
//! null-or-value [`OptionalType`] over union storage, and the [`list`], [`map`] and
//! [`struct`](r#struct) modules the nested types (each type its own data type,
//! field and scalar). Add more following the rules in `CLAUDE.md`.
//!
//! Every layer converts to and from its Apache Arrow equivalent (`to_arrow` /
//! `from_arrow`): a data type mirrors an [`arrow_schema::DataType`], a field an
//! [`arrow_schema::Field`], and a scalar a one-element [`arrow_array`] array. The
//! `arrow-schema` and `arrow-array` subset crates are re-exported so downstream code
//! uses the exact versions this crate was built against. Skipped inputs (such as
//! dropped Arrow field metadata) are logged behind the off-by-default `log` cargo
//! feature, mirroring `yggdryl-core`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays logging-free by default and pays no runtime
/// cost). Submodules reach it via `crate::log_event!` thanks to the re-export
/// below.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        ::log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

/// The Apache Arrow array layer (`arrow-array`), re-exported so downstream code and
/// the scalar `to_arrow` / `from_arrow` surface share one version.
pub use arrow_array;
/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the data type / field `to_arrow` / `from_arrow` surface share one version.
pub use arrow_schema;

mod data_type_id;
mod error;
mod raw_data_type;
mod raw_field;
mod raw_scalar;

mod data_type;
mod field;
mod scalar;

mod logical;
mod nested;
mod primitive;
mod raw_logical;
mod raw_nested;

pub use data_type_id::DataTypeId;
pub use error::DataError;
pub use raw_data_type::RawDataType;
pub use raw_field::RawField;
pub use raw_scalar::RawScalar;

pub use data_type::DataType;
pub use field::Field;
pub use scalar::Scalar;

pub use logical::Logical;
pub use nested::Nested;
pub use primitive::Primitive;
pub use raw_logical::RawLogical;
pub use raw_nested::RawNested;

pub mod integer;
pub mod list;
pub mod map;
pub mod null;
pub mod optional;
pub mod r#struct;
pub mod union;

pub use list::{List, ListField, ListScalar, ListType, RawList};
pub use map::{Map, MapField, MapScalar, MapType, RawMap};
pub use null::{Null, NullField, NullScalar};
pub use optional::{Optional, OptionalField, OptionalScalar, OptionalType, RawOptional};
pub use r#struct::{RawStruct, Struct, StructField, StructScalar, StructType};
pub use union::{RawUnion, Union, UnionField, UnionType};

pub use integer::{
    Int16, Int16Field, Int16Scalar, Int32, Int32Field, Int32Scalar, Int64, Int64Field, Int64Scalar,
    Int8, Int8Field, Int8Scalar, UInt16, UInt16Field, UInt16Scalar, UInt32, UInt32Field,
    UInt32Scalar, UInt64, UInt64Field, UInt64Scalar, UInt8, UInt8Field, UInt8Scalar,
};
