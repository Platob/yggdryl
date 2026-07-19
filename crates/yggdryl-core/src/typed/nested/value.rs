//! [`Value`] — the **erased single element** of any [`Column`](super::Column): a tagged union over
//! every concrete element type, the runtime parallel of the compile-time-typed native scalars.
//!
//! [`Column::get`](super::Column::get) returns one; a [`StructScalar`](super::StructScalar) row is a
//! `Vec<Value>`. It carries the value **by value** (an owned `Vec<u8>` / `String` for the byte types,
//! a nested [`StructScalar`] for a struct row), so a `Value` outlives the column it came from.

use crate::datatype_id::DataTypeId;
use crate::typed::fixedbyte::I256;
use crate::typed::nested::{ListScalar, MapScalar, StructScalar};

/// One erased, possibly-null typed value — the element a [`Column::get`](super::Column::get) yields
/// and a [`StructScalar`] row holds. `PartialEq` compares like variants (float `NaN` is unequal to
/// itself, as usual); it is deliberately **not** `Eq` / `Hash`, because it can hold a float.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A null / absent element (of any type).
    Null,
    /// A signed 8-bit integer element.
    Int8(i8),
    /// An unsigned 8-bit integer element.
    UInt8(u8),
    /// A signed 16-bit integer element.
    Int16(i16),
    /// An unsigned 16-bit integer element.
    UInt16(u16),
    /// A signed 32-bit integer element.
    Int32(i32),
    /// An unsigned 32-bit integer element.
    UInt32(u32),
    /// A signed 64-bit integer element.
    Int64(i64),
    /// An unsigned 64-bit integer element.
    UInt64(u64),
    /// A signed 128-bit integer element.
    Int128(i128),
    /// An unsigned 128-bit integer element.
    UInt128(u128),
    /// A 32-bit float element.
    Float32(f32),
    /// A 64-bit float element.
    Float64(f64),
    /// A boolean element.
    Bool(bool),
    /// A 32-bit fixed-point decimal element (its unscaled `i32`).
    Decimal32(i32),
    /// A 64-bit fixed-point decimal element (its unscaled `i64`).
    Decimal64(i64),
    /// A 128-bit fixed-point decimal element (its unscaled `i128`).
    Decimal128(i128),
    /// A 256-bit fixed-point decimal element (its unscaled [`I256`]).
    Decimal256(I256),
    /// A binary element (owned) — the value of any binary column (`Binary` / `LargeBinary` /
    /// `FixedBinary`).
    Binary(Vec<u8>),
    /// A UTF-8 string element (owned) — the value of any string column (`Utf8` / `LargeUtf8` /
    /// `FixedUtf8`).
    Utf8(String),
    /// A **nested struct row** — the element of a struct column (a [`StructScalar`]).
    Row(StructScalar),
    /// A **list element** — the element of a list column (a [`ListScalar`]: its children as owned
    /// [`Value`]s).
    List(ListScalar),
    /// A **map element** — the element of a map column (a [`MapScalar`]: its key→value entries as
    /// owned [`Value`]s).
    Map(MapScalar),
}

impl Value {
    /// Whether this is the [`Null`](Value::Null) element.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The element [`DataTypeId`] this value carries — [`Null`](DataTypeId::Null) for
    /// [`Value::Null`], the matching primitive / byte / nested id otherwise. The runtime dtype of an
    /// erased value, used (among others) to name both sides of a `set_any_scalar_at` type mismatch.
    pub fn data_type_id(&self) -> DataTypeId {
        match self {
            Value::Null => DataTypeId::Null,
            Value::Int8(_) => DataTypeId::I8,
            Value::UInt8(_) => DataTypeId::U8,
            Value::Int16(_) => DataTypeId::I16,
            Value::UInt16(_) => DataTypeId::U16,
            Value::Int32(_) => DataTypeId::I32,
            Value::UInt32(_) => DataTypeId::U32,
            Value::Int64(_) => DataTypeId::I64,
            Value::UInt64(_) => DataTypeId::U64,
            Value::Int128(_) => DataTypeId::I128,
            Value::UInt128(_) => DataTypeId::U128,
            Value::Float32(_) => DataTypeId::F32,
            Value::Float64(_) => DataTypeId::F64,
            Value::Bool(_) => DataTypeId::Bool,
            Value::Decimal32(_) => DataTypeId::Decimal32,
            Value::Decimal64(_) => DataTypeId::Decimal64,
            Value::Decimal128(_) => DataTypeId::Decimal128,
            Value::Decimal256(_) => DataTypeId::Decimal256,
            Value::Binary(_) => DataTypeId::Binary,
            Value::Utf8(_) => DataTypeId::Utf8,
            Value::Row(_) => DataTypeId::Struct,
            Value::List(_) => DataTypeId::List,
            Value::Map(_) => DataTypeId::Map,
        }
    }
}
