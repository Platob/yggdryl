//! # yggdryl-dtype
//!
//! The Apache Arrow-centralized data-type layer for yggdryl, built on top of
//! `yggdryl-core`. It defines the physical and logical **data types** of the model —
//! the first of the three data layers (`yggdryl-dtype`, `yggdryl-field`,
//! `yggdryl-scalar`), each concern its own crate, so the concrete types share one
//! naming convention across the layers (a `yggdryl_dtype::Int64Type` describes the
//! type, a `yggdryl_field::Int64Field` names a column of it, a
//! `yggdryl_scalar::Int64Scalar` holds one value of it).
//!
//! The type system comes in two layers of traits, plus categories, each re-exported
//! at the crate root. Every base trait carries the bare namespace name and every
//! typed refinement the `Typed…` prefix:
//!
//! - The **untyped base** [`DataType`] — the FFI-facing descriptor.
//! - The **typed** [`TypedDataType`], parameterised by a native Rust type `T` (the
//!   codec bridging a `T` to and from Arrow bytes). It is the generic *factory*: it
//!   builds the type's [`default_value`](TypedDataType::default_value), and — via the
//!   `yggdryl-field` / `yggdryl-scalar` extension traits — its field and scalar.
//! - The **categories** [`Primitive`], [`Logical`] / [`TypedLogical`] and [`Nested`]
//!   / [`TypedNested`] describing a type's shape.
//!
//! Concrete types live in per-family modules — the [`integer`] module holds every
//! signed and unsigned integer, the [`binary`] module the variable-length byte
//! sequence, the [`null`] module the storage-free null type, the [`union`] module
//! the union type, the [`optional`] module the logical null-or-value [`OptionalType`]
//! over union storage, and the [`serie`], [`map`] and [`struct`](r#struct) modules
//! the nested types. Add more following the rules in `CLAUDE.md`.
//!
//! Every type converts to and from the [`arrow_schema::DataType`] it mirrors
//! (`to_arrow` / `from_arrow`). The `arrow-schema` subset crate is re-exported so
//! downstream code uses the exact version this crate was built against. No code path
//! here skips, defaults or mutates shared state, so this crate carries no `log`
//! feature — the upper layers log their own skips.

/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the `to_arrow` / `from_arrow` surface share one version.
pub use arrow_schema;
/// The yggdryl foundation layer (`yggdryl-core`), re-exported so downstream code
/// reaches the [`IOError`](yggdryl_core::IOError) wrapped by [`DataError::Io`] at
/// the exact version this crate was built against.
pub use yggdryl_core;

mod data_type;
mod data_type_id;
mod error;
mod typed_data_type;

mod logical;
mod nested;
mod primitive;
mod typed_logical;
mod typed_nested;

pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use error::DataError;
pub use typed_data_type::TypedDataType;

pub use logical::Logical;
pub use nested::Nested;
pub use primitive::Primitive;
pub use typed_logical::TypedLogical;
pub use typed_nested::TypedNested;

pub mod binary;
pub mod integer;
pub mod map;
pub mod null;
pub mod optional;
pub mod serie;
pub mod r#struct;
pub mod union;

pub use binary::BinaryType;
pub use map::{Map, MapType, TypedMap};
pub use null::NullType;
pub use optional::{Optional, OptionalType, TypedOptional};
pub use r#struct::{Struct, StructType, TypedStruct};
pub use serie::{Serie, SerieType, TypedSerie};
pub use union::{TypedUnion, Union, UnionType};

pub use integer::{
    Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
