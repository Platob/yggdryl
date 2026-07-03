//! # yggdryl-dtype
//!
//! The Apache Arrow-centralized data-type layer for yggdryl, built on top of
//! `yggdryl-core`. It defines the physical and logical **data types** of the model —
//! the first of the three data layers (`yggdryl-dtype`, `yggdryl-field`,
//! `yggdryl-scalar`), each concern its own crate, so the concrete types share one
//! bare name across the layers (a `yggdryl_dtype::Int64` describes the type, a
//! `yggdryl_field::Int64` names a column of it, a `yggdryl_scalar::Int64` holds one
//! value of it).
//!
//! The type system comes in two layers of traits, plus categories, each re-exported
//! at the crate root:
//!
//! - The **untyped base** [`RawDataType`] — the FFI-facing descriptor.
//! - The **typed** [`DataType`], parameterised by a native Rust type `T` (the codec
//!   bridging a `T` to and from Arrow bytes). The default *scalar* of a type lives
//!   upstream in `yggdryl-scalar` (its `DefaultScalar` trait), keeping this crate
//!   scalar-free.
//! - The **categories** [`Primitive`], [`Logical`] and [`Nested`] describing a
//!   type's shape.
//!
//! Concrete types live in per-family modules — the [`integer`] module holds every
//! signed and unsigned integer, the [`binary`] module the variable-length byte
//! sequence, the [`null`] module the storage-free null type, the [`union`] module
//! the union type, the [`optional`] module the logical null-or-value [`Optional`]
//! over union storage, and the [`list`], [`map`] and [`struct`](r#struct) modules
//! the nested types. Add more following the rules in `CLAUDE.md`.
//!
//! Every type converts to and from the [`arrow_schema::DataType`] it mirrors
//! (`to_arrow` / `from_arrow`). The `arrow-schema` subset crate is re-exported so
//! downstream code uses the exact version this crate was built against. No code
//! path here skips, defaults or mutates shared state, so this crate carries no
//! `log` feature — the upper layers (`yggdryl-field` drops unmodeled Arrow field
//! metadata) log their own skips.

/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the `to_arrow` / `from_arrow` surface share one version.
pub use arrow_schema;
/// The yggdryl foundation layer (`yggdryl-core`), re-exported so downstream code
/// reaches the [`IOError`](yggdryl_core::IOError) wrapped by [`DataError::Io`] at
/// the exact version this crate was built against.
pub use yggdryl_core;

mod data_type_id;
mod error;
mod raw_data_type;

mod data_type;

mod logical;
mod nested;
mod primitive;
mod raw_logical;
mod raw_nested;

pub use data_type_id::DataTypeId;
pub use error::DataError;
pub use raw_data_type::RawDataType;

pub use data_type::DataType;

pub use logical::Logical;
pub use nested::Nested;
pub use primitive::Primitive;
pub use raw_logical::RawLogical;
pub use raw_nested::RawNested;

pub mod binary;
pub mod integer;
pub mod list;
pub mod map;
pub mod null;
pub mod optional;
pub mod r#struct;
pub mod union;

pub use binary::Binary;
pub use list::{List, RawList, TypedList};
pub use map::{Map, RawMap, TypedMap};
pub use null::Null;
pub use optional::{Optional, RawOptional, TypedOptional};
pub use r#struct::{RawStruct, Struct, TypedStruct};
pub use union::{RawUnion, TypedUnion, Union};

pub use integer::{Int16, Int32, Int64, Int8, UInt16, UInt32, UInt64, UInt8};
