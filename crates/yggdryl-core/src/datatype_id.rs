//! `dtype` — the primitive **element data types** a byte region can be interpreted as.
//!
//! [`DataTypeId`] is a compact `#[repr(u16)]` int enum naming every native fixed-width primitive
//! (`bool`, the signed/unsigned integers `i8`…`u128`, the floats `f32`/`f64`). It round-trips
//! through a `u16` — the value a source stores in its [`Headers`](crate::headers::Headers) as the
//! `Type-Id` — so the byte layer knows its **element width** (the size the typed accessors and
//! the vectorized aggregations step by), can compute an element count, and can safely widen /
//! shrink a region between widths.

/// A **primitive element data type** — the interpretation of a fixed-width value in a byte region.
/// A plain `#[repr(u16)]` int enum (`Unknown = 0` is the default "raw bytes" state): it converts
/// to/from a `u16`, keys a map, sits in a set, and travels over a wire.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DataTypeId {
    /// Unknown / raw bytes — no declared element type (the default).
    #[default]
    Unknown = 0,
    /// A boolean — 1 byte in storage, 1 bit logically.
    Bool = 1,
    /// Signed 8-bit integer.
    I8 = 2,
    /// Unsigned 8-bit integer.
    U8 = 3,
    /// Signed 16-bit integer.
    I16 = 4,
    /// Unsigned 16-bit integer.
    U16 = 5,
    /// Signed 32-bit integer.
    I32 = 6,
    /// Unsigned 32-bit integer.
    U32 = 7,
    /// Signed 64-bit integer.
    I64 = 8,
    /// Unsigned 64-bit integer.
    U64 = 9,
    /// Signed 128-bit integer.
    I128 = 10,
    /// Unsigned 128-bit integer.
    U128 = 11,
    /// 32-bit IEEE-754 float.
    F32 = 12,
    /// 64-bit IEEE-754 float.
    F64 = 13,
    /// 32-bit fixed-point **decimal** — a signed `i32` unscaled value (precision/scale in metadata).
    Decimal32 = 14,
    /// 64-bit fixed-point **decimal** — a signed `i64` unscaled value.
    Decimal64 = 15,
    /// 128-bit fixed-point **decimal** — a signed `i128` unscaled value.
    Decimal128 = 16,
    /// 256-bit fixed-point **decimal** — a signed `I256` unscaled value.
    Decimal256 = 17,
    /// **Variable-length binary** — an `i32`-offsets + data byte layout (`Vec<u8>` elements).
    Binary = 18,
    /// **Variable-length UTF-8** string — the same offsets + data layout (`String` elements).
    Utf8 = 19,
    /// **Fixed-length binary** — a fixed byte width per element (the width in the field metadata).
    FixedBinary = 20,
    /// **Fixed-length UTF-8** — a fixed byte width per element (the width in the field metadata).
    FixedUtf8 = 21,
}

impl DataTypeId {
    /// Every non-`Unknown` type, in id order — the canonical set (used by tests and registries).
    pub const ALL: [DataTypeId; 21] = [
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
        DataTypeId::Utf8,
        DataTypeId::FixedBinary,
        DataTypeId::FixedUtf8,
    ];

    /// The `u16` discriminant — what a source stores in its headers.
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// The type for a `u16` discriminant, or [`Unknown`](DataTypeId::Unknown) for an unrecognized
    /// value (total, never panics — a foreign/newer id degrades to raw bytes).
    pub fn from_u16(value: u16) -> DataTypeId {
        match value {
            1 => DataTypeId::Bool,
            2 => DataTypeId::I8,
            3 => DataTypeId::U8,
            4 => DataTypeId::I16,
            5 => DataTypeId::U16,
            6 => DataTypeId::I32,
            7 => DataTypeId::U32,
            8 => DataTypeId::I64,
            9 => DataTypeId::U64,
            10 => DataTypeId::I128,
            11 => DataTypeId::U128,
            12 => DataTypeId::F32,
            13 => DataTypeId::F64,
            14 => DataTypeId::Decimal32,
            15 => DataTypeId::Decimal64,
            16 => DataTypeId::Decimal128,
            17 => DataTypeId::Decimal256,
            18 => DataTypeId::Binary,
            19 => DataTypeId::Utf8,
            20 => DataTypeId::FixedBinary,
            21 => DataTypeId::FixedUtf8,
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
            DataTypeId::Utf8 => "utf8",
            DataTypeId::FixedBinary => "fixed_binary",
            DataTypeId::FixedUtf8 => "fixed_utf8",
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

    /// The **storage width** of one element in bytes (`i32` → 4, `i128` → 16, `bool` → 1); `0` for
    /// [`Unknown`](DataTypeId::Unknown) (raw bytes have no fixed element width).
    pub fn byte_size(self) -> u64 {
        match self {
            DataTypeId::Unknown => 0,
            DataTypeId::Bool | DataTypeId::I8 | DataTypeId::U8 => 1,
            DataTypeId::I16 | DataTypeId::U16 => 2,
            DataTypeId::I32 | DataTypeId::U32 | DataTypeId::F32 => 4,
            DataTypeId::I64 | DataTypeId::U64 | DataTypeId::F64 => 8,
            DataTypeId::I128 | DataTypeId::U128 => 16,
            DataTypeId::Decimal32 => 4,
            DataTypeId::Decimal64 => 8,
            DataTypeId::Decimal128 => 16,
            DataTypeId::Decimal256 => 32,
            // Variable-length + fixed-size byte types have no id-derivable element width (a
            // fixed-size type's width lives in the field metadata).
            DataTypeId::Binary
            | DataTypeId::Utf8
            | DataTypeId::FixedBinary
            | DataTypeId::FixedUtf8 => 0,
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

    /// Whether this is a **binary** byte type (`Binary` / `FixedBinary`).
    pub fn is_binary(self) -> bool {
        matches!(self, DataTypeId::Binary | DataTypeId::FixedBinary)
    }

    /// Whether this is a **UTF-8 string** type (`Utf8` / `FixedUtf8`).
    pub fn is_utf8(self) -> bool {
        matches!(self, DataTypeId::Utf8 | DataTypeId::FixedUtf8)
    }

    /// Whether this is a **variable-length** type (`Binary` / `Utf8`) — an offsets + data layout.
    pub fn is_variable_length(self) -> bool {
        matches!(self, DataTypeId::Binary | DataTypeId::Utf8)
    }

    /// Whether this is an integer type (`bool` is **not** counted as an integer).
    pub fn is_integer(self) -> bool {
        matches!(
            self,
            DataTypeId::I8
                | DataTypeId::U8
                | DataTypeId::I16
                | DataTypeId::U16
                | DataTypeId::I32
                | DataTypeId::U32
                | DataTypeId::I64
                | DataTypeId::U64
                | DataTypeId::I128
                | DataTypeId::U128
        )
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
        matches!(self, DataTypeId::F32 | DataTypeId::F64)
    }

    /// Whether this is a fixed-point **decimal** type (`decimal32`…`decimal256`).
    pub fn is_decimal(self) -> bool {
        matches!(
            self,
            DataTypeId::Decimal32
                | DataTypeId::Decimal64
                | DataTypeId::Decimal128
                | DataTypeId::Decimal256
        )
    }

    /// Whether this is the boolean type.
    pub fn is_bool(self) -> bool {
        self == DataTypeId::Bool
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
