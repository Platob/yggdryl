//! [`Value`] — the **erased single element** of any [`Column`](super::Column): a tagged union over
//! every concrete element type, the runtime parallel of the compile-time-typed native scalars.
//!
//! [`Column::get`](super::Column::get) returns one; a [`StructScalar`](super::StructScalar) row is a
//! `Vec<Value>`. It carries the value **by value** (an owned `Vec<u8>` / `String` for the byte types,
//! a nested [`StructScalar`] for a struct row), so a `Value` outlives the column it came from.

use crate::typed::fixedbyte::I256;
use crate::typed::nested::StructScalar;

/// One erased, possibly-null typed value — the element a [`Column::get`](super::Column::get) yields
/// and a [`StructScalar`] row holds. `PartialEq` compares like variants (float `NaN` is unequal to
/// itself, as usual); it is deliberately **not** `Eq` / `Hash`, because it can hold a float.
///
// DESIGN: `List` / `Map` element forms (a `Vec<Value>` / a key→value map) land with their carriers
// in a later phase — this enum grows a variant then, so downstream matches keep a wildcard arm.
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
}

impl Value {
    /// Whether this is the [`Null`](Value::Null) element.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}
