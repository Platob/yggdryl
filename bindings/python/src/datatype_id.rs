//! The `yggdryl.datatype_id` submodule — the primitive **element data types** a byte region can be
//! interpreted as.
//!
//! Mirrors [`yggdryl_core::datatype_id::DataTypeId`]: a compact int enum naming every native
//! fixed-width primitive (`bool`, the signed/unsigned integers `i8`…`u128`, the floats
//! `f32`/`f64`, and the fixed-point decimals `decimal32`…`decimal256`). It round-trips through a
//! `u16` — the value a source stores in its `Headers` as the
//! `Type-Id` — so the byte layer knows its **element width** (the size the typed accessors and the
//! vectorized aggregations step by), can compute an element count, and can safely widen / shrink a
//! region between widths. Hashable and frozen like an int enum.
//!
//! Every method is one or two lines over `yggdryl_core`.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`. `wrong_self_convention`: `as_u16` keeps the core method name, but a `#[pymethods]`
// receiver cannot take `self` by value, so it borrows.
#![allow(clippy::useless_conversion, clippy::wrong_self_convention)]

use pyo3::prelude::*;

use yggdryl_core::datatype_id;

/// A **primitive element data type** — the interpretation of a value in a byte region, with the
/// same wire-stable numeric values as the core. The ids are laid out in **per-category bands** with
/// reserved gaps — `0x0000` special (`Unknown = 0`), `0x0010` boolean (`Bool = 16`), `0x0100`
/// integers (`I8 = 256` … `U128 = 265`), `0x0200` floats (`F32 = 513`, `F64 = 514`), `0x0300`
/// decimals (`Decimal32 = 768` … `Decimal256 = 771`), `0x0500` binary (`Binary = 1280`,
/// `FixedBinary = 1296`), `0x0600` UTF-8 (`Utf8 = 1536`, `FixedUtf8 = 1552`) — so `DataTypeId.I32 ==
/// 260` and `int(DataTypeId.I32) == 260` (`category()` names the band). `Unknown` is the default
/// "raw bytes" state. The four byte columns are **not** fixed-width (their `byte_size()` is `0`; a
/// fixed-size column's width lives in its field metadata). Hashable and frozen like an int enum.
#[pyclass(module = "yggdryl.datatype_id", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTypeId {
    /// Unknown / raw bytes — no declared element type (the default). Value `0` (band `0x0000`).
    Unknown = 0x0000,
    /// A boolean — 1 byte in storage, 1 bit logically. Value `16` (band `0x0010`).
    Bool = 0x0010,
    /// Signed 8-bit integer. Value `256` (integer band `0x0100`).
    I8 = 0x0100,
    /// Unsigned 8-bit integer. Value `257`.
    U8 = 0x0101,
    /// Signed 16-bit integer. Value `258`.
    I16 = 0x0102,
    /// Unsigned 16-bit integer. Value `259`.
    U16 = 0x0103,
    /// Signed 32-bit integer. Value `260`.
    I32 = 0x0104,
    /// Unsigned 32-bit integer. Value `261`.
    U32 = 0x0105,
    /// Signed 64-bit integer. Value `262`.
    I64 = 0x0106,
    /// Unsigned 64-bit integer. Value `263`.
    U64 = 0x0107,
    /// Signed 128-bit integer. Value `264`.
    I128 = 0x0108,
    /// Unsigned 128-bit integer. Value `265`.
    U128 = 0x0109,
    /// 32-bit IEEE-754 float. Value `513` (float band `0x0200`; `0x0200` reserved for `f16`).
    F32 = 0x0201,
    /// 64-bit IEEE-754 float. Value `514`.
    F64 = 0x0202,
    /// 32-bit fixed-point **decimal** — a signed `i32` unscaled value (precision/scale in metadata).
    /// Value `768` (decimal band `0x0300`).
    Decimal32 = 0x0300,
    /// 64-bit fixed-point **decimal** — a signed `i64` unscaled value. Value `769`.
    Decimal64 = 0x0301,
    /// 128-bit fixed-point **decimal** — a signed `i128` unscaled value. Value `770`.
    Decimal128 = 0x0302,
    /// 256-bit fixed-point **decimal** — a signed `I256` unscaled value. Value `771`.
    Decimal256 = 0x0303,
    /// **Variable-length binary** — an offsets + data byte blob (`bytes` elements). Value `1280`
    /// (binary band `0x0500`).
    Binary = 0x0500,
    /// **Fixed-length binary** — a fixed per-column byte width (`bytes` elements). Value `1296`.
    FixedBinary = 0x0510,
    /// **Variable-length UTF-8** string — the same offsets + data layout (`str` elements). Value
    /// `1536` (UTF-8 band `0x0600`).
    Utf8 = 0x0600,
    /// **Fixed-length UTF-8** string — a fixed per-column byte width (`str` elements). Value `1552`.
    FixedUtf8 = 0x0610,
}

impl From<DataTypeId> for datatype_id::DataTypeId {
    fn from(id: DataTypeId) -> Self {
        match id {
            DataTypeId::Unknown => datatype_id::DataTypeId::Unknown,
            DataTypeId::Bool => datatype_id::DataTypeId::Bool,
            DataTypeId::I8 => datatype_id::DataTypeId::I8,
            DataTypeId::U8 => datatype_id::DataTypeId::U8,
            DataTypeId::I16 => datatype_id::DataTypeId::I16,
            DataTypeId::U16 => datatype_id::DataTypeId::U16,
            DataTypeId::I32 => datatype_id::DataTypeId::I32,
            DataTypeId::U32 => datatype_id::DataTypeId::U32,
            DataTypeId::I64 => datatype_id::DataTypeId::I64,
            DataTypeId::U64 => datatype_id::DataTypeId::U64,
            DataTypeId::I128 => datatype_id::DataTypeId::I128,
            DataTypeId::U128 => datatype_id::DataTypeId::U128,
            DataTypeId::F32 => datatype_id::DataTypeId::F32,
            DataTypeId::F64 => datatype_id::DataTypeId::F64,
            DataTypeId::Decimal32 => datatype_id::DataTypeId::Decimal32,
            DataTypeId::Decimal64 => datatype_id::DataTypeId::Decimal64,
            DataTypeId::Decimal128 => datatype_id::DataTypeId::Decimal128,
            DataTypeId::Decimal256 => datatype_id::DataTypeId::Decimal256,
            DataTypeId::Binary => datatype_id::DataTypeId::Binary,
            DataTypeId::Utf8 => datatype_id::DataTypeId::Utf8,
            DataTypeId::FixedBinary => datatype_id::DataTypeId::FixedBinary,
            DataTypeId::FixedUtf8 => datatype_id::DataTypeId::FixedUtf8,
        }
    }
}

impl From<datatype_id::DataTypeId> for DataTypeId {
    fn from(id: datatype_id::DataTypeId) -> Self {
        match id {
            datatype_id::DataTypeId::Unknown => DataTypeId::Unknown,
            datatype_id::DataTypeId::Bool => DataTypeId::Bool,
            datatype_id::DataTypeId::I8 => DataTypeId::I8,
            datatype_id::DataTypeId::U8 => DataTypeId::U8,
            datatype_id::DataTypeId::I16 => DataTypeId::I16,
            datatype_id::DataTypeId::U16 => DataTypeId::U16,
            datatype_id::DataTypeId::I32 => DataTypeId::I32,
            datatype_id::DataTypeId::U32 => DataTypeId::U32,
            datatype_id::DataTypeId::I64 => DataTypeId::I64,
            datatype_id::DataTypeId::U64 => DataTypeId::U64,
            datatype_id::DataTypeId::I128 => DataTypeId::I128,
            datatype_id::DataTypeId::U128 => DataTypeId::U128,
            datatype_id::DataTypeId::F32 => DataTypeId::F32,
            datatype_id::DataTypeId::F64 => DataTypeId::F64,
            datatype_id::DataTypeId::Decimal32 => DataTypeId::Decimal32,
            datatype_id::DataTypeId::Decimal64 => DataTypeId::Decimal64,
            datatype_id::DataTypeId::Decimal128 => DataTypeId::Decimal128,
            datatype_id::DataTypeId::Decimal256 => DataTypeId::Decimal256,
            datatype_id::DataTypeId::Binary => DataTypeId::Binary,
            datatype_id::DataTypeId::Utf8 => DataTypeId::Utf8,
            datatype_id::DataTypeId::FixedBinary => DataTypeId::FixedBinary,
            datatype_id::DataTypeId::FixedUtf8 => DataTypeId::FixedUtf8,
            // The core enum is `#[non_exhaustive]`; a newer/foreign id degrades to raw bytes.
            _ => DataTypeId::Unknown,
        }
    }
}

#[pymethods]
impl DataTypeId {
    /// The `u16` discriminant — what a source stores in its headers (`Unknown = 0`, … `F64 = 13`).
    fn as_u16(&self) -> u16 {
        datatype_id::DataTypeId::from(*self).as_u16()
    }

    /// The type for a `u16` discriminant, or [`Unknown`](DataTypeId::Unknown) for an unrecognized
    /// value (total, never fails — a foreign/newer id degrades to raw bytes).
    #[staticmethod]
    fn from_u16(value: u16) -> DataTypeId {
        datatype_id::DataTypeId::from_u16(value).into()
    }

    /// The stable lowercase token (`"i32"`, `"f64"`, `"bool"`, `"unknown"`).
    fn name(&self) -> &'static str {
        datatype_id::DataTypeId::from(*self).name()
    }

    /// The type named by `token` (`"i32"`, `"f64"`, …, case-insensitive), or `None` when the name
    /// is not a known type.
    #[staticmethod]
    fn from_name(token: &str) -> Option<DataTypeId> {
        datatype_id::DataTypeId::from_name(token).map(DataTypeId::from)
    }

    /// The **storage width** of one element in bytes (`i32` → 4, `i128` → 16, `bool` → 1); `0` for
    /// [`Unknown`](DataTypeId::Unknown) (raw bytes have no fixed element width).
    fn byte_size(&self) -> u64 {
        datatype_id::DataTypeId::from(*self).byte_size()
    }

    /// The **logical bit width** of one element — `bool` is `1`, every other fixed type is
    /// `byte_size() * 8`, and [`Unknown`](DataTypeId::Unknown) is `0`.
    fn bit_size(&self) -> u64 {
        datatype_id::DataTypeId::from(*self).bit_size()
    }

    /// Whether this is an integer type (`bool` is **not** counted as an integer).
    fn is_integer(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_integer()
    }

    /// Whether this is a **signed** numeric type (the signed integers and the floats).
    fn is_signed(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_signed()
    }

    /// Whether this is a floating-point type (`f32` / `f64`).
    fn is_float(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_float()
    }

    /// Whether this is the boolean type.
    fn is_bool(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_bool()
    }

    /// Whether this is a fixed-width type (everything except [`Unknown`](DataTypeId::Unknown) and
    /// the four byte columns).
    fn is_fixed_width(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_fixed_width()
    }

    /// Whether this is a **binary** byte column (`Binary` / `FixedBinary`).
    fn is_binary(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_binary()
    }

    /// Whether this is a **UTF-8 string** column (`Utf8` / `FixedUtf8`).
    fn is_utf8(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_utf8()
    }

    /// Whether this is a **variable-length** byte column (`Binary` / `Utf8`) — an offsets + data
    /// layout (a fixed-size column packs at a fixed stride instead).
    fn is_variable_length(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_variable_length()
    }

    /// The **category** this type's band belongs to, as a lowercase name (`"integer"`, `"float"`,
    /// `"decimal"`, `"binary"`, `"utf8"`, `"boolean"`, `"null"`, plus the reserved `"temporal"` /
    /// `"nested"`).
    fn category(&self) -> &'static str {
        datatype_id::DataTypeId::from(*self).category().name()
    }

    /// Whether this is a **numeric** type — an integer, a float, or a decimal (not `bool`, not a
    /// byte/string type).
    fn is_numeric(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_numeric()
    }

    /// Whether this is a **byte / string** type (binary or UTF-8).
    fn is_byte_like(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_byte_like()
    }

    /// Whether this is a **fixed-size** byte / string type (`FixedBinary` / `FixedUtf8`).
    fn is_fixed_size(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_fixed_size()
    }

    /// Whether this is a **temporal** type (the reserved date / time / timestamp band).
    fn is_temporal(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_temporal()
    }

    /// Whether this is a **nested / composite** type (the reserved struct / list / map band).
    fn is_nested(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_nested()
    }

    /// How many whole elements of this type fit in `bytes` — `bytes / byte_size()`, or `0` for
    /// [`Unknown`](DataTypeId::Unknown) (raw bytes have no element count).
    fn element_count(&self, bytes: u64) -> u64 {
        datatype_id::DataTypeId::from(*self).element_count(bytes)
    }

    /// The canonical lowercase name (so `str(dtype)` reads like the core `Display`).
    fn __str__(&self) -> &'static str {
        datatype_id::DataTypeId::from(*self).name()
    }

    /// The `u16` id as an integer index (so it can index a sequence / `operator.index`) — the
    /// index counterpart of the pyo3-provided `__int__`.
    fn __index__(&self) -> u16 {
        datatype_id::DataTypeId::from(*self).as_u16()
    }
}

/// Populates the `datatype_id` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DataTypeId>()?;
    Ok(())
}
