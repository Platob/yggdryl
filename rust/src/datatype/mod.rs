//! Owned, allocation-conscious Arrow logical datatypes.
//!
//! [`DataType`] mirrors every `arrow_schema::DataType` variant while keeping
//! Arrow out of the public value model. Scalar values are inline and nested
//! values use shared immutable allocations, making clones of nested schemas
//! allocation-free.

use std::sync::Arc;

use crate::Field;

mod arrow;
mod comparison;
mod compatibility;
mod default;
mod floating;
mod geospatial;
mod integer;
mod nested;
mod parser;
mod scalar;
pub(crate) mod serde;
mod temporal;

pub(crate) use arrow::arrow_data_type_to_ffi;

pub use crate::enums::{TimeUnit, UnionMode};
pub(crate) use default::{
    default_value_for_field, preflight_schema, preflight_schema_shape, value_is_logically_null,
};
pub use geospatial::GeospatialType;
pub use nested::{DictionaryType, Fields, MapType, RunEndEncodedType, UnionFields};

/// An allocation-conscious logical datatype with complete Arrow 59.2 parity.
///
/// Scalar variants are inline. Nested children use `Arc`, so cloning a
/// datatype never allocates. Cache state belongs to [`Field`], not this value.
///
/// Parameterized variants remain public for ergonomic pattern matching and
/// Arrow parity. Caller-created values can therefore bypass constructors and
/// temporarily contain invalid parameters. Prefer validated constructors such
/// as [`Self::time`], [`Self::decimal`], and [`Self::map`]. Arrow
/// projection, structural serialization, and [`Self::validate`] reject every
/// invalid state before it crosses an interoperability boundary.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum DataType {
    /// Null values.
    Null,
    /// Boolean values.
    Boolean,
    /// Signed 8-bit integers.
    Int8,
    /// Signed 16-bit integers.
    Int16,
    /// Signed 32-bit integers.
    Int32,
    /// Signed 64-bit integers.
    Int64,
    /// Unsigned 8-bit integers.
    UInt8,
    /// Unsigned 16-bit integers.
    UInt16,
    /// Unsigned 32-bit integers.
    UInt32,
    /// Unsigned 64-bit integers.
    UInt64,
    /// IEEE 16-bit floating point.
    Float16,
    /// IEEE 32-bit floating point.
    Float32,
    /// IEEE 64-bit floating point.
    Float64,
    /// Timestamp unit and optional IANA timezone or fixed offset.
    Timestamp(TimeUnit, Option<crate::Timezone>),
    /// Days since the Unix epoch.
    Date32,
    /// Milliseconds since the Unix epoch representing whole days.
    Date64,
    /// 32-bit time of day; seconds and milliseconds are valid.
    Time32(TimeUnit),
    /// 64-bit time of day; microseconds and nanoseconds are valid.
    Time64(TimeUnit),
    /// Elapsed time.
    Duration(TimeUnit),
    /// Calendar interval.
    Interval(TimeUnit),
    /// Variable-width binary data with 32-bit offsets.
    Binary,
    /// Fixed-width binary data.
    FixedSizeBinary(i32),
    /// Variable-width binary data with 64-bit offsets.
    LargeBinary,
    /// Binary view layout.
    BinaryView,
    /// UTF-8 with 32-bit offsets.
    Utf8,
    /// UTF-8 with 64-bit offsets.
    LargeUtf8,
    /// UTF-8 view layout.
    Utf8View,
    /// Variable list with 32-bit offsets.
    List(Arc<Field>),
    /// Variable list-view with 32-bit offsets.
    ListView(Arc<Field>),
    /// Fixed-length list.
    FixedSizeList(Arc<Field>, i32),
    /// Variable list with 64-bit offsets.
    LargeList(Arc<Field>),
    /// Variable list-view with 64-bit offsets.
    LargeListView(Arc<Field>),
    /// Ordered struct fields.
    Struct(Fields),
    /// Tagged union fields and layout mode.
    Union(UnionFields, UnionMode),
    /// Dictionary key and value types.
    Dictionary(Arc<DictionaryType>),
    /// Exact decimal backed by 32 bits.
    Decimal32 { precision: u8, scale: i8 },
    /// Exact decimal backed by 64 bits.
    Decimal64 { precision: u8, scale: i8 },
    /// Exact decimal backed by 128 bits.
    Decimal128 { precision: u8, scale: i8 },
    /// Exact decimal backed by 256 bits.
    Decimal256 { precision: u8, scale: i8 },
    /// Arrow map entries and key-order flag.
    Map(Arc<MapType>),
    /// Run-end encoding child fields.
    RunEndEncoded(Arc<RunEndEncodedType>),
    /// Self-describing semi-structured values.
    ///
    /// A variant value is a [`crate::Value`] - a tree that declares its own
    /// types per value - so the type takes no parameters: shredding is a
    /// physical layout, not part of the logical type. Bare `variant` is this
    /// type; `variant(...)` with members stays the dense-union input sugar,
    /// and the parenthesis is what disambiguates.
    Variant,
    /// Planar geospatial features, carried as Well-Known Binary.
    Geometry(Arc<GeospatialType>),
    /// Geospatial features on a sphere or spheroid, carried as WKB.
    Geography(Arc<GeospatialType>),
}

#[cfg(test)]
mod tests;
