//! The `yggdryl.datatype_id` namespace's [`DataTypeId`] — the primitive **element data types** a
//! byte region can be interpreted as.
//!
//! Mirrors `yggdryl_core::datatype_id::DataTypeId`, a compact `#[repr(u16)]` int enum naming every native
//! fixed-width primitive (`bool`, the signed/unsigned integers `i8`…`u128`, the floats
//! `f32`/`f64`, the fixed-point `decimal32`…`decimal256`) plus the **byte types** (variable-length
//! `binary` / `utf8`, the large-offset `large_binary` / `large_utf8`, and fixed-size `fixed_binary` /
//! `fixed_utf8`). napi cannot attach methods to a
//! bare enum, so — like the core — the type is
//! exposed as a thin `#[napi]` **class** carrying its `u16` `id`: each variant is a named static
//! factory (`DataTypeId.I64()`), and the width / classification helpers are methods
//! (`asU16` / `name` / `byteSize` / `bitSize` / `isInteger` / … / `elementCount` / `toString`),
//! with the static parsers `fromU16` / `fromName`. Every method is a one- or two-line delegation
//! to `yggdryl_core`; a bad `fromName` token surfaces as a thrown `Error` carrying the core's
//! guided text.

use napi_derive::napi;

use yggdryl_core::datatype_id as core;

/// A **primitive element data type** — the interpretation of a value in a byte region (`Unknown` is
/// the default "raw bytes" state). A thin value over the core's `#[repr(u16)]` id: it round-trips
/// through a `u16` (the value a source stores in its `Headers` as `Type-Id`), so the byte layer
/// knows its element width, can compute an element count, and can widen / shrink a region between
/// widths. Beyond the fixed-width numeric / decimal types it also names the **byte types**
/// (variable-length `binary` / `utf8`, the large-offset `large_binary` / `large_utf8`, fixed-size
/// `fixed_binary` / `fixed_utf8`). Equatable and stringly named; the id keys a map or travels over a
/// wire.
#[napi(namespace = "datatype_id")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DataTypeId {
    pub(crate) inner: core::DataTypeId,
}

#[napi(namespace = "datatype_id")]
impl DataTypeId {
    /// Builds a data type from its **`u16` id** (`8` → `I64`); an unrecognized id degrades to
    /// [`Unknown`](DataTypeId::unknown) (total, never throws). The generic entry — `fromU16` is
    /// its named alias.
    #[napi(constructor)]
    pub fn new(id: u16) -> Self {
        DataTypeId {
            inner: core::DataTypeId::from_u16(id),
        }
    }

    // ---- variant factories (one per type) ----------------------------------------------

    /// Unknown / raw bytes — no declared element type (the default, id `0`).
    #[napi(factory, js_name = "Unknown")]
    pub fn variant_unknown() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Unknown,
        }
    }

    /// A boolean — 1 byte in storage, 1 bit logically (id `1`).
    #[napi(factory, js_name = "Bool")]
    pub fn variant_bool() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Bool,
        }
    }

    /// Signed 8-bit integer (id `2`).
    #[napi(factory, js_name = "I8")]
    pub fn variant_i8() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::I8,
        }
    }

    /// Unsigned 8-bit integer (id `3`).
    #[napi(factory, js_name = "U8")]
    pub fn variant_u8() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::U8,
        }
    }

    /// Signed 16-bit integer (id `4`).
    #[napi(factory, js_name = "I16")]
    pub fn variant_i16() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::I16,
        }
    }

    /// Unsigned 16-bit integer (id `5`).
    #[napi(factory, js_name = "U16")]
    pub fn variant_u16() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::U16,
        }
    }

    /// Signed 32-bit integer (id `6`).
    #[napi(factory, js_name = "I32")]
    pub fn variant_i32() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::I32,
        }
    }

    /// Unsigned 32-bit integer (id `7`).
    #[napi(factory, js_name = "U32")]
    pub fn variant_u32() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::U32,
        }
    }

    /// Signed 64-bit integer (id `8`).
    #[napi(factory, js_name = "I64")]
    pub fn variant_i64() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::I64,
        }
    }

    /// Unsigned 64-bit integer (id `9`).
    #[napi(factory, js_name = "U64")]
    pub fn variant_u64() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::U64,
        }
    }

    /// Signed 128-bit integer (id `10`).
    #[napi(factory, js_name = "I128")]
    pub fn variant_i128() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::I128,
        }
    }

    /// Unsigned 128-bit integer (id `11`).
    #[napi(factory, js_name = "U128")]
    pub fn variant_u128() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::U128,
        }
    }

    /// 32-bit IEEE-754 float (id `12`).
    #[napi(factory, js_name = "F32")]
    pub fn variant_f32() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::F32,
        }
    }

    /// 64-bit IEEE-754 float (id `13`).
    #[napi(factory, js_name = "F64")]
    pub fn variant_f64() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::F64,
        }
    }

    /// 32-bit fixed-point decimal over an unscaled `i32` (id `14`).
    #[napi(factory, js_name = "Decimal32")]
    pub fn variant_decimal32() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Decimal32,
        }
    }

    /// 64-bit fixed-point decimal over an unscaled `i64` (id `15`).
    #[napi(factory, js_name = "Decimal64")]
    pub fn variant_decimal64() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Decimal64,
        }
    }

    /// 128-bit fixed-point decimal over an unscaled `i128` (id `16`).
    #[napi(factory, js_name = "Decimal128")]
    pub fn variant_decimal128() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Decimal128,
        }
    }

    /// 256-bit fixed-point decimal over an unscaled `I256` (id `17`).
    #[napi(factory, js_name = "Decimal256")]
    pub fn variant_decimal256() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Decimal256,
        }
    }

    /// **Variable-length binary** — an arbitrary byte blob per element (id `18`).
    #[napi(factory, js_name = "Binary")]
    pub fn variant_binary() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Binary,
        }
    }

    /// **Variable-length UTF-8** — a string per element (id `19`).
    #[napi(factory, js_name = "Utf8")]
    pub fn variant_utf8() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::Utf8,
        }
    }

    /// **Large variable-length binary** — an arbitrary byte blob per element with **`i64` offsets**
    /// (the same layout as `Binary`, for a column whose total data bytes exceed the `i32` offset
    /// range; id `0x0502`).
    #[napi(factory, js_name = "LargeBinary")]
    pub fn variant_large_binary() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::LargeBinary,
        }
    }

    /// **Large variable-length UTF-8** — a string per element with **`i64` offsets** (the same
    /// layout as `Utf8`, for a column whose total data bytes exceed the `i32` offset range;
    /// id `0x0602`).
    #[napi(factory, js_name = "LargeUtf8")]
    pub fn variant_large_utf8() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::LargeUtf8,
        }
    }

    /// **Fixed-size binary** — a byte blob at a per-column fixed byte width (id `20`).
    #[napi(factory, js_name = "FixedBinary")]
    pub fn variant_fixed_binary() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::FixedBinary,
        }
    }

    /// **Fixed-size UTF-8** — a string at a per-column fixed byte width (id `21`).
    #[napi(factory, js_name = "FixedUtf8")]
    pub fn variant_fixed_utf8() -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::FixedUtf8,
        }
    }

    // ---- id / name round-trips ---------------------------------------------------------

    /// The `u16` discriminant — what a source stores in its headers.
    #[napi(getter)]
    pub fn id(&self) -> u16 {
        self.inner.as_u16()
    }

    /// The `u16` discriminant — the method form of the `id` getter.
    #[napi]
    pub fn as_u16(&self) -> u16 {
        self.inner.as_u16()
    }

    /// The data type for a `u16` discriminant, or [`Unknown`](DataTypeId::unknown) for an
    /// unrecognized value (total, never throws — a foreign/newer id degrades to raw bytes).
    #[napi(factory)]
    pub fn from_u16(value: u16) -> DataTypeId {
        DataTypeId {
            inner: core::DataTypeId::from_u16(value),
        }
    }

    /// The stable lowercase token (`"i32"`, `"f64"`, `"bool"`, `"unknown"`).
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The data type named by `token` (`"i32"`, `"f64"`, …, case-insensitive), or throws a guided
    /// `Error` naming the accepted tokens for an unrecognized name.
    #[napi(factory)]
    pub fn from_name(token: String) -> napi::Result<DataTypeId> {
        core::DataTypeId::from_name(&token)
            .map(|inner| DataTypeId { inner })
            .ok_or_else(|| {
                napi::Error::from_reason(format!(
                    "unknown data type name {token:?}: expected one of unknown, bool, i8, u8, \
                     i16, u16, i32, u32, i64, u64, i128, u128, f32, f64, decimal32, decimal64, \
                     decimal128, decimal256, binary, utf8, large_binary, large_utf8, fixed_binary, \
                     fixed_utf8"
                ))
            })
    }

    // ---- widths + classification -------------------------------------------------------

    /// The **storage width** of one element in bytes (`i32` → 4, `i128` → 16, `bool` → 1); `0`
    /// for [`Unknown`](DataTypeId::unknown). An `i64` (a JS number).
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The **logical bit width** of one element — `bool` is `1`, every other fixed type is
    /// `byteSize * 8`, and [`Unknown`](DataTypeId::unknown) is `0`. An `i64` (a JS number).
    #[napi]
    pub fn bit_size(&self) -> i64 {
        self.inner.bit_size() as i64
    }

    /// Whether this is an integer type (`bool` is **not** counted as an integer).
    #[napi]
    pub fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }

    /// Whether this is a **signed** numeric type (the signed integers and the floats).
    #[napi]
    pub fn is_signed(&self) -> bool {
        self.inner.is_signed()
    }

    /// Whether this is a floating-point type (`f32` / `f64`).
    #[napi]
    pub fn is_float(&self) -> bool {
        self.inner.is_float()
    }

    /// Whether this is the boolean type.
    #[napi]
    pub fn is_bool(&self) -> bool {
        self.inner.is_bool()
    }

    /// Whether this is a fixed-width type (everything except [`Unknown`](DataTypeId::unknown) and the
    /// byte types).
    #[napi]
    pub fn is_fixed_width(&self) -> bool {
        self.inner.is_fixed_width()
    }

    /// Whether this is a **binary** byte type (`Binary` / `FixedBinary`).
    #[napi]
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    /// Whether this is a **UTF-8 string** type (`Utf8` / `FixedUtf8`).
    #[napi]
    pub fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }

    /// Whether this is a **variable-length** byte type (`Binary` / `Utf8`).
    #[napi]
    pub fn is_variable_length(&self) -> bool {
        self.inner.is_variable_length()
    }

    /// The **category** this type's band belongs to, as a lowercase name (`"integer"`, `"float"`,
    /// `"decimal"`, `"binary"`, `"utf8"`, `"boolean"`, `"null"`, plus the reserved `"temporal"` and
    /// the nested `"struct"` / `"list"` / `"map"`).
    #[napi]
    pub fn category(&self) -> String {
        self.inner.category().name().to_string()
    }

    /// Whether this is a **numeric** type — an integer, a float, or a decimal (not `bool`, not a
    /// byte/string type).
    #[napi]
    pub fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }

    /// Whether this is a **byte / string** type (binary or UTF-8).
    #[napi]
    pub fn is_byte_like(&self) -> bool {
        self.inner.is_byte_like()
    }

    /// Whether this is a **fixed-size** byte / string type (`FixedBinary` / `FixedUtf8`).
    #[napi]
    pub fn is_fixed_size(&self) -> bool {
        self.inner.is_fixed_size()
    }

    /// Whether this is a **large** variable-length byte / string type (`LargeBinary` / `LargeUtf8`) —
    /// the offsets + data layout with **`i64` offsets**, for data past the `i32` offset range.
    #[napi]
    pub fn is_large(&self) -> bool {
        self.inner.is_large()
    }

    /// Whether this is a **temporal** type (the reserved date / time / timestamp band).
    #[napi]
    pub fn is_temporal(&self) -> bool {
        self.inner.is_temporal()
    }

    /// Whether this is a **nested / composite** type (the reserved struct / list / map band).
    #[napi]
    pub fn is_nested(&self) -> bool {
        self.inner.is_nested()
    }

    /// How many whole elements of this type fit in `bytes` — `bytes / byteSize`, or `0` for
    /// [`Unknown`](DataTypeId::unknown). `bytes` and the result are `i64`s (JS numbers); a
    /// negative `bytes` counts as `0`.
    #[napi]
    pub fn element_count(&self, bytes: i64) -> i64 {
        self.inner.element_count(u64::try_from(bytes).unwrap_or(0)) as i64
    }

    // ---- value semantics ---------------------------------------------------------------

    /// Identity equality — equal iff they name the same element type.
    #[napi]
    pub fn equals(&self, other: &DataTypeId) -> bool {
        self.inner == other.inner
    }

    /// The stable lowercase token — the same string `name()` returns.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}
