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

/// A **primitive element data type** — the interpretation of a fixed-width value in a byte region,
/// with the same wire-stable numeric values as the core (`Unknown = 0`, `Bool = 1`, … `F64 = 13`,
/// `Decimal32 = 14`, … `Decimal256 = 17`), so `DataTypeId.I32 == 6` and `int(DataTypeId.I32) == 6`.
/// `Unknown` is the default "raw bytes" state. Hashable and frozen like an int enum.
#[pyclass(module = "yggdryl.datatype_id", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTypeId {
    /// Unknown / raw bytes — no declared element type (the default). Value `0`.
    Unknown = 0,
    /// A boolean — 1 byte in storage, 1 bit logically. Value `1`.
    Bool = 1,
    /// Signed 8-bit integer. Value `2`.
    I8 = 2,
    /// Unsigned 8-bit integer. Value `3`.
    U8 = 3,
    /// Signed 16-bit integer. Value `4`.
    I16 = 4,
    /// Unsigned 16-bit integer. Value `5`.
    U16 = 5,
    /// Signed 32-bit integer. Value `6`.
    I32 = 6,
    /// Unsigned 32-bit integer. Value `7`.
    U32 = 7,
    /// Signed 64-bit integer. Value `8`.
    I64 = 8,
    /// Unsigned 64-bit integer. Value `9`.
    U64 = 9,
    /// Signed 128-bit integer. Value `10`.
    I128 = 10,
    /// Unsigned 128-bit integer. Value `11`.
    U128 = 11,
    /// 32-bit IEEE-754 float. Value `12`.
    F32 = 12,
    /// 64-bit IEEE-754 float. Value `13`.
    F64 = 13,
    /// 32-bit fixed-point **decimal** — a signed `i32` unscaled value (precision/scale in metadata).
    /// Value `14`.
    Decimal32 = 14,
    /// 64-bit fixed-point **decimal** — a signed `i64` unscaled value. Value `15`.
    Decimal64 = 15,
    /// 128-bit fixed-point **decimal** — a signed `i128` unscaled value. Value `16`.
    Decimal128 = 16,
    /// 256-bit fixed-point **decimal** — a signed `I256` unscaled value. Value `17`.
    Decimal256 = 17,
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

    /// Whether this is a fixed-width type (everything except [`Unknown`](DataTypeId::Unknown)).
    fn is_fixed_width(&self) -> bool {
        datatype_id::DataTypeId::from(*self).is_fixed_width()
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
