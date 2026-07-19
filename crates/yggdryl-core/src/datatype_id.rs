//! `datatype_id` — the primitive **element data types** a byte region can be interpreted as.
//!
//! [`DataTypeId`] is a compact `#[repr(u16)]` int enum naming every element type — the numeric
//! primitives (`bool`, the signed/unsigned integers `i8`…`u128`, the floats `f32`/`f64`, the
//! fixed-point decimals) and the byte/string types (variable-length and fixed-size binary / UTF-8).
//! It round-trips through a `u16` — the value a source stores in its [`Headers`](crate::headers::Headers)
//! as the `Type-Id` — so the byte layer knows its **element width** (the size the typed accessors and
//! the vectorized aggregations step by), can compute an element count, and can safely widen / shrink
//! a region between widths.
//!
//! The ids are laid out in **per-category bands** with reserved gaps ([`DataTypeCategory`]), so a new
//! width slots in beside its neighbours without renumbering an existing id:
//!
//! | band | category | members |
//! |---|---|---|
//! | `0x0000` | [`Null`](DataTypeCategory::Null) | `Unknown` |
//! | `0x0010` | [`Boolean`](DataTypeCategory::Boolean) | `Bool` |
//! | `0x0100` | [`Integer`](DataTypeCategory::Integer) | `I8`…`U128` |
//! | `0x0200` | [`Float`](DataTypeCategory::Float) | `F32`, `F64` (`0x0200` reserved for `F16`) |
//! | `0x0300` | [`Decimal`](DataTypeCategory::Decimal) | `Decimal32`…`Decimal256` |
//! | `0x0400` | [`Temporal`](DataTypeCategory::Temporal) | *(reserved — date / time / timestamp)* |
//! | `0x0500` | [`Binary`](DataTypeCategory::Binary) | `Binary`, `LargeBinary`, `FixedBinary` |
//! | `0x0600` | [`Utf8`](DataTypeCategory::Utf8) | `Utf8`, `LargeUtf8`, `FixedUtf8` |
//! | `0x0700` | [`Nested`](DataTypeCategory::Nested) | `Struct`, `List`, `Map` |

/// The **broad family** a [`DataTypeId`] belongs to — one per band. `category()` returns it, and the
/// coarse predicates (`is_integer` / `is_float` / …) are band membership checks against it.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataTypeCategory {
    /// `Unknown` — raw bytes, no declared element type.
    Null = 0,
    /// The boolean type.
    Boolean = 1,
    /// The signed / unsigned integers (`i8`…`u128`).
    Integer = 2,
    /// The IEEE-754 floats (`f32` / `f64`).
    Float = 3,
    /// The fixed-point decimals (`decimal32`…`decimal256`).
    Decimal = 4,
    /// Date / time / timestamp types *(reserved band)*.
    Temporal = 5,
    /// Binary byte blobs (`binary` / `fixed_binary` / `large_binary`).
    Binary = 6,
    /// UTF-8 strings (`utf8` / `fixed_utf8` / `large_utf8`).
    Utf8 = 7,
    /// Composite types — `struct` / `list` / `map` *(reserved band)*.
    Nested = 8,
}

impl DataTypeCategory {
    /// The stable lowercase token (`"integer"`, `"utf8"`, `"null"`).
    pub fn name(self) -> &'static str {
        match self {
            DataTypeCategory::Null => "null",
            DataTypeCategory::Boolean => "boolean",
            DataTypeCategory::Integer => "integer",
            DataTypeCategory::Float => "float",
            DataTypeCategory::Decimal => "decimal",
            DataTypeCategory::Temporal => "temporal",
            DataTypeCategory::Binary => "binary",
            DataTypeCategory::Utf8 => "utf8",
            DataTypeCategory::Nested => "nested",
        }
    }
}

impl core::fmt::Display for DataTypeCategory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A **primitive element data type** — the interpretation of a value in a byte region. A plain
/// `#[repr(u16)]` int enum laid out in per-category bands (`Unknown = 0` is the default "raw bytes"
/// state): it converts to/from a `u16`, keys a map, sits in a set, and travels over a wire.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DataTypeId {
    /// Unknown / raw bytes — no declared element type (the default). Band `0x0000`.
    #[default]
    Unknown = 0x0000,

    // ---- boolean band (0x0010) --------------------------------------------------------
    /// A boolean — 1 byte in storage, 1 bit logically.
    Bool = 0x0010,

    // ---- integer band (0x0100) --------------------------------------------------------
    /// Signed 8-bit integer.
    I8 = 0x0100,
    /// Unsigned 8-bit integer.
    U8 = 0x0101,
    /// Signed 16-bit integer.
    I16 = 0x0102,
    /// Unsigned 16-bit integer.
    U16 = 0x0103,
    /// Signed 32-bit integer.
    I32 = 0x0104,
    /// Unsigned 32-bit integer.
    U32 = 0x0105,
    /// Signed 64-bit integer.
    I64 = 0x0106,
    /// Unsigned 64-bit integer.
    U64 = 0x0107,
    /// Signed 128-bit integer.
    I128 = 0x0108,
    /// Unsigned 128-bit integer.
    U128 = 0x0109,

    // ---- float band (0x0200; 0x0200 reserved for F16) ---------------------------------
    /// 32-bit IEEE-754 float.
    F32 = 0x0201,
    /// 64-bit IEEE-754 float.
    F64 = 0x0202,

    // ---- decimal band (0x0300) --------------------------------------------------------
    /// 32-bit fixed-point **decimal** — a signed `i32` unscaled value (precision/scale in metadata).
    Decimal32 = 0x0300,
    /// 64-bit fixed-point **decimal** — a signed `i64` unscaled value.
    Decimal64 = 0x0301,
    /// 128-bit fixed-point **decimal** — a signed `i128` unscaled value.
    Decimal128 = 0x0302,
    /// 256-bit fixed-point **decimal** — a signed `I256` unscaled value.
    Decimal256 = 0x0303,

    // ---- binary band (0x0500; 0x0503 reserved for a future large-fixed slot) ----------
    /// **Variable-length binary** — an `i32`-offsets + data byte layout (`Vec<u8>` elements).
    Binary = 0x0500,
    /// **Large variable-length binary** — the `Binary` layout with **`i64` offsets** (Arrow's
    /// `LargeBinary`), for a column whose total data bytes exceed the `i32` offset range.
    LargeBinary = 0x0502,
    /// **Fixed-length binary** — a fixed byte width per element (the width in the field metadata).
    FixedBinary = 0x0510,

    // ---- utf8 band (0x0600; 0x0603 reserved for a future large-fixed slot) -------------
    /// **Variable-length UTF-8** string — the same offsets + data layout (`String` elements).
    Utf8 = 0x0600,
    /// **Large variable-length UTF-8** string — the `Utf8` layout with **`i64` offsets** (Arrow's
    /// `LargeUtf8`), for a column whose total data bytes exceed the `i32` offset range.
    LargeUtf8 = 0x0602,
    /// **Fixed-length UTF-8** — a fixed byte width per element (the width in the field metadata).
    FixedUtf8 = 0x0610,

    // ---- nested band (0x0700) ---------------------------------------------------------
    /// A **struct** — a heterogeneous, ordered set of named child columns (the project's
    /// "table"-like holder). Its element is a row; its children are themselves any [`DataTypeId`].
    Struct = 0x0700,
    /// A **list** — a variable-length sequence of one child element type *(reserved this phase — the
    /// id slots the band; the carrier lands in a later phase)*.
    List = 0x0710,
    /// A **map** — an ordered set of key→value entries *(reserved this phase — the id slots the band;
    /// the carrier lands in a later phase)*.
    Map = 0x0720,
}

impl DataTypeId {
    /// Every non-`Unknown` type, in id order — the canonical set (used by tests and registries).
    pub const ALL: [DataTypeId; 26] = [
        DataTypeId::Bool,
        DataTypeId::I8,
        DataTypeId::U8,
        DataTypeId::I16,
        DataTypeId::U16,
        DataTypeId::I32,
        DataTypeId::U32,
        DataTypeId::I64,
        DataTypeId::U64,
        DataTypeId::I128,
        DataTypeId::U128,
        DataTypeId::F32,
        DataTypeId::F64,
        DataTypeId::Decimal32,
        DataTypeId::Decimal64,
        DataTypeId::Decimal128,
        DataTypeId::Decimal256,
        DataTypeId::Binary,
        DataTypeId::LargeBinary,
        DataTypeId::FixedBinary,
        DataTypeId::Utf8,
        DataTypeId::LargeUtf8,
        DataTypeId::FixedUtf8,
        DataTypeId::Struct,
        DataTypeId::List,
        DataTypeId::Map,
    ];

    /// The `u16` discriminant — what a source stores in its headers.
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// The type for a `u16` discriminant, or [`Unknown`](DataTypeId::Unknown) for an unrecognized
    /// value (total, never panics — a foreign/newer id degrades to raw bytes).
    pub fn from_u16(value: u16) -> DataTypeId {
        match value {
            0x0010 => DataTypeId::Bool,
            0x0100 => DataTypeId::I8,
            0x0101 => DataTypeId::U8,
            0x0102 => DataTypeId::I16,
            0x0103 => DataTypeId::U16,
            0x0104 => DataTypeId::I32,
            0x0105 => DataTypeId::U32,
            0x0106 => DataTypeId::I64,
            0x0107 => DataTypeId::U64,
            0x0108 => DataTypeId::I128,
            0x0109 => DataTypeId::U128,
            0x0201 => DataTypeId::F32,
            0x0202 => DataTypeId::F64,
            0x0300 => DataTypeId::Decimal32,
            0x0301 => DataTypeId::Decimal64,
            0x0302 => DataTypeId::Decimal128,
            0x0303 => DataTypeId::Decimal256,
            0x0500 => DataTypeId::Binary,
            0x0502 => DataTypeId::LargeBinary,
            0x0510 => DataTypeId::FixedBinary,
            0x0600 => DataTypeId::Utf8,
            0x0602 => DataTypeId::LargeUtf8,
            0x0610 => DataTypeId::FixedUtf8,
            0x0700 => DataTypeId::Struct,
            0x0710 => DataTypeId::List,
            0x0720 => DataTypeId::Map,
            _ => DataTypeId::Unknown,
        }
    }

    /// The stable lowercase token (`"i32"`, `"f64"`, `"bool"`, `"unknown"`).
    pub fn name(self) -> &'static str {
        match self {
            DataTypeId::Unknown => "unknown",
            DataTypeId::Bool => "bool",
            DataTypeId::I8 => "i8",
            DataTypeId::U8 => "u8",
            DataTypeId::I16 => "i16",
            DataTypeId::U16 => "u16",
            DataTypeId::I32 => "i32",
            DataTypeId::U32 => "u32",
            DataTypeId::I64 => "i64",
            DataTypeId::U64 => "u64",
            DataTypeId::I128 => "i128",
            DataTypeId::U128 => "u128",
            DataTypeId::F32 => "f32",
            DataTypeId::F64 => "f64",
            DataTypeId::Decimal32 => "decimal32",
            DataTypeId::Decimal64 => "decimal64",
            DataTypeId::Decimal128 => "decimal128",
            DataTypeId::Decimal256 => "decimal256",
            DataTypeId::Binary => "binary",
            DataTypeId::LargeBinary => "large_binary",
            DataTypeId::Utf8 => "utf8",
            DataTypeId::LargeUtf8 => "large_utf8",
            DataTypeId::FixedBinary => "fixed_binary",
            DataTypeId::FixedUtf8 => "fixed_utf8",
            DataTypeId::Struct => "struct",
            DataTypeId::List => "list",
            DataTypeId::Map => "map",
        }
    }

    /// The type named by `token` (`"i32"`, `"f64"`, …, case-insensitive), or `None`.
    pub fn from_name(token: &str) -> Option<DataTypeId> {
        let lower = token.trim().to_ascii_lowercase();
        DataTypeId::ALL
            .into_iter()
            .find(|t| t.name() == lower)
            .or(match lower.as_str() {
                "unknown" | "" => Some(DataTypeId::Unknown),
                _ => None,
            })
    }

    /// The **broad family** this type belongs to — its band's [`DataTypeCategory`].
    pub fn category(self) -> DataTypeCategory {
        match (self as u16) >> 8 {
            0x00 => {
                if self == DataTypeId::Bool {
                    DataTypeCategory::Boolean
                } else {
                    DataTypeCategory::Null
                }
            }
            0x01 => DataTypeCategory::Integer,
            0x02 => DataTypeCategory::Float,
            0x03 => DataTypeCategory::Decimal,
            0x04 => DataTypeCategory::Temporal,
            0x05 => DataTypeCategory::Binary,
            0x06 => DataTypeCategory::Utf8,
            0x07 => DataTypeCategory::Nested,
            _ => DataTypeCategory::Null,
        }
    }

    /// The **storage width** of one element in bytes (`i32` → 4, `i128` → 16, `bool` → 1); `0` for
    /// [`Unknown`](DataTypeId::Unknown) and the byte/string types (raw bytes / a variable or
    /// field-metadata width have no id-derivable fixed element width).
    pub fn byte_size(self) -> u64 {
        match self {
            DataTypeId::Bool | DataTypeId::I8 | DataTypeId::U8 => 1,
            DataTypeId::I16 | DataTypeId::U16 => 2,
            DataTypeId::I32 | DataTypeId::U32 | DataTypeId::F32 => 4,
            DataTypeId::I64 | DataTypeId::U64 | DataTypeId::F64 => 8,
            DataTypeId::I128 | DataTypeId::U128 => 16,
            DataTypeId::Decimal32 => 4,
            DataTypeId::Decimal64 => 8,
            DataTypeId::Decimal128 => 16,
            DataTypeId::Decimal256 => 32,
            // Unknown + the variable-length / fixed-size byte types + the nested composites have no
            // id-derivable element width (a fixed-size type's width lives in the field metadata; a
            // nested type's layout is in its children, not a single fixed stride).
            DataTypeId::Unknown
            | DataTypeId::Binary
            | DataTypeId::LargeBinary
            | DataTypeId::Utf8
            | DataTypeId::LargeUtf8
            | DataTypeId::FixedBinary
            | DataTypeId::FixedUtf8
            | DataTypeId::Struct
            | DataTypeId::List
            | DataTypeId::Map => 0,
        }
    }

    /// The **logical bit width** of one element — `bool` is `1`, every other fixed type is
    /// `byte_size() * 8`, and [`Unknown`](DataTypeId::Unknown) is `0`.
    pub fn bit_size(self) -> u64 {
        match self {
            DataTypeId::Unknown => 0,
            DataTypeId::Bool => 1,
            other => other.byte_size() * 8,
        }
    }

    /// Whether this type has an **id-derivable fixed element width** — the numeric / bool / decimal
    /// types (`byte_size() > 0`). The variable-length and fixed-size byte types (`Binary` / `Utf8` /
    /// `FixedBinary` / `FixedUtf8`) return `false` (their width, if any, is field metadata).
    pub fn is_fixed_width(self) -> bool {
        self.byte_size() > 0
    }

    /// Whether this is a **binary** byte type (the [`Binary`](DataTypeCategory::Binary) band —
    /// `Binary` / `FixedBinary`).
    pub fn is_binary(self) -> bool {
        self.category() == DataTypeCategory::Binary
    }

    /// Whether this is a **UTF-8 string** type (the [`Utf8`](DataTypeCategory::Utf8) band — `Utf8` /
    /// `FixedUtf8`).
    pub fn is_utf8(self) -> bool {
        self.category() == DataTypeCategory::Utf8
    }

    /// Whether this is a **byte / string** type (binary or UTF-8) — a blob whose width is not
    /// id-derivable.
    pub fn is_byte_like(self) -> bool {
        matches!(
            self.category(),
            DataTypeCategory::Binary | DataTypeCategory::Utf8
        )
    }

    /// Whether this is a **fixed-size** byte / string type (`FixedBinary` / `FixedUtf8`) — packed at
    /// a per-column byte width (in the field metadata), no offsets buffer.
    pub fn is_fixed_size(self) -> bool {
        matches!(self, DataTypeId::FixedBinary | DataTypeId::FixedUtf8)
    }

    /// Whether this is a **large** variable-length byte / string type (`LargeBinary` / `LargeUtf8`) —
    /// the offsets + data layout with **`i64` offsets** (Arrow's `Large*`), for data past the `i32`
    /// offset range.
    pub fn is_large(self) -> bool {
        matches!(self, DataTypeId::LargeBinary | DataTypeId::LargeUtf8)
    }

    /// Whether this is a **variable-length** byte / string type — an offsets + data layout (`Binary`
    /// / `Utf8` with `i32` offsets, `LargeBinary` / `LargeUtf8` with `i64` offsets), i.e. a byte-like
    /// type that is not fixed-size.
    pub fn is_variable_length(self) -> bool {
        self.is_byte_like() && !self.is_fixed_size()
    }

    /// Whether this is a **temporal** type (the reserved [`Temporal`](DataTypeCategory::Temporal)
    /// band — date / time / timestamp).
    pub fn is_temporal(self) -> bool {
        self.category() == DataTypeCategory::Temporal
    }

    /// Whether this is a **nested / composite** type (the reserved [`Nested`](DataTypeCategory::Nested)
    /// band — struct / list / map).
    pub fn is_nested(self) -> bool {
        self.category() == DataTypeCategory::Nested
    }

    /// Whether this is an integer type (`bool` is **not** counted as an integer).
    pub fn is_integer(self) -> bool {
        self.category() == DataTypeCategory::Integer
    }

    /// Whether this is a **signed** numeric type (the signed integers, the floats, and the decimals).
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            DataTypeId::I8
                | DataTypeId::I16
                | DataTypeId::I32
                | DataTypeId::I64
                | DataTypeId::I128
                | DataTypeId::F32
                | DataTypeId::F64
        ) || self.is_decimal()
    }

    /// Whether this is a floating-point type (`f32` / `f64`).
    pub fn is_float(self) -> bool {
        self.category() == DataTypeCategory::Float
    }

    /// Whether this is a fixed-point **decimal** type (`decimal32`…`decimal256`).
    pub fn is_decimal(self) -> bool {
        self.category() == DataTypeCategory::Decimal
    }

    /// Whether this is the boolean type.
    pub fn is_bool(self) -> bool {
        self == DataTypeId::Bool
    }

    /// Whether this is a **numeric** type — an integer, a float, or a decimal (not `bool`, not a
    /// byte/string type). The set the vectorized numeric reductions run over.
    pub fn is_numeric(self) -> bool {
        matches!(
            self.category(),
            DataTypeCategory::Integer | DataTypeCategory::Float | DataTypeCategory::Decimal
        )
    }

    /// How many whole elements of this type fit in `bytes` — `bytes / byte_size()`, or `0` for
    /// [`Unknown`](DataTypeId::Unknown) (raw bytes have no element count).
    pub fn element_count(self, bytes: u64) -> u64 {
        match self.byte_size() {
            0 => 0,
            width => bytes / width,
        }
    }
}

impl core::fmt::Display for DataTypeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}
