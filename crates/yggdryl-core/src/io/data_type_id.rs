//! [`DataTypeId`] — the centralized, fine-grained **integer identity** of every concrete data
//! type, and the single axis the category drill-down (`is_integer`, `is_utf8`, …) reduces to.

use super::DataTypeCategory;

#[cfg(feature = "arrow")]
use crate::io::fixed::temporal::{TimeUnit, Tz};

/// The stable **integer identity** of a concrete data type — the one place the type space is
/// enumerated, and the source of truth every `is_*` predicate reduces to.
///
/// It is `#[repr(u16)]` with the discriminants laid out so each category is a **contiguous
/// integer range** with gaps reserved for future types, so a predicate is one or two `u16`
/// comparisons (`lo <= id <= hi`) instead of a `match` on the concrete type — cheap, inlinable,
/// and adding a type in a reserved slot never touches a predicate. The whole `0x0001..=0x00FF`
/// band is fixed-width, `0x0100..=0x01FF` is variable-length.
///
/// | range (inclusive) | contents |
/// | --- | --- |
/// | `0x0000` | the null type |
/// | `0x0001..=0x000F` | *reserved* — fixed-width non-numeric (future `Boolean`, …) |
/// | `0x0010..=0x001F` | unsigned integers (`U8`…`U256`) |
/// | `0x0020..=0x002F` | signed integers (`I8`…`I256`) |
/// | `0x0030..=0x003F` | floats (`F16`, `F32`, `F64`) |
/// | `0x0040..=0x0047` | fixed-size binary (`FixedBinary`) |
/// | `0x0048..=0x004F` | fixed-size utf8 (`FixedUtf8`) |
/// | `0x0050..=0x005F` | decimals (`D32`, `D64`, `D128`, `D256`) |
/// | `0x0060..=0x007F` | temporal (`Date32/64`, `Time32/64`, `Ts32/64/96`, `Duration32/64`) |
/// | `0x0080..=0x00FF` | *reserved* — future fixed-width |
/// | `0x0100..=0x0107` | variable binary (`Binary`, `LargeBinary`) |
/// | `0x0108..=0x010F` | variable utf8 (`Utf8`, `LargeUtf8`) |
/// | `0x0110..=0x01FF` | *reserved* — future variable-length (views, …) |
/// | `0x0200..=0x020F` | struct (`Struct`), with reserved gaps |
/// | `0x0210..=0x021F` | list (`List`; reserved `LargeList`/`FixedSizeList`) |
/// | `0x0220..=0x022F` | map (`Map`) |
/// | `0x0230..=0x02FF` | *reserved* — future nested / composite |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum DataTypeId {
    /// The null type.
    Null = 0x0000,

    /// Unsigned 8-bit integer.
    U8 = 0x0010,
    /// Unsigned 16-bit integer.
    U16 = 0x0011,
    /// Unsigned 32-bit integer.
    U32 = 0x0012,
    /// Unsigned 64-bit integer.
    U64 = 0x0013,
    /// Unsigned 96-bit integer.
    U96 = 0x0014,
    /// Unsigned 128-bit integer.
    U128 = 0x0015,
    /// Unsigned 256-bit integer.
    U256 = 0x0016,

    /// Signed 8-bit integer.
    I8 = 0x0020,
    /// Signed 16-bit integer.
    I16 = 0x0021,
    /// Signed 32-bit integer.
    I32 = 0x0022,
    /// Signed 64-bit integer.
    I64 = 0x0023,
    /// Signed 96-bit integer.
    I96 = 0x0024,
    /// Signed 128-bit integer.
    I128 = 0x0025,
    /// Signed 256-bit integer.
    I256 = 0x0026,

    /// IEEE-754 half-precision float.
    F16 = 0x0030,
    /// IEEE-754 single-precision float.
    F32 = 0x0031,
    /// IEEE-754 double-precision float.
    F64 = 0x0032,

    /// Fixed-size opaque binary (`N` bytes per value).
    FixedBinary = 0x0040,
    /// Fixed-size UTF-8 string (`N` bytes per value).
    FixedUtf8 = 0x0048,

    /// 32-bit scaled decimal (`i32` coefficient; Arrow `Decimal32`).
    D32 = 0x0050,
    /// 64-bit scaled decimal (`i64` coefficient; Arrow `Decimal64`).
    D64 = 0x0051,
    /// 128-bit scaled decimal (`i128` coefficient; Arrow `Decimal128`).
    D128 = 0x0052,
    /// 256-bit scaled decimal (256-bit coefficient; Arrow `Decimal256`).
    D256 = 0x0053,

    /// Calendar date as `i32` days since the epoch (Arrow `Date32`).
    Date32 = 0x0060,
    /// Calendar date as `i64` milliseconds since the epoch (Arrow `Date64`).
    Date64 = 0x0061,
    /// Time of day as `i32` (seconds / milliseconds; Arrow `Time32`).
    Time32 = 0x0064,
    /// Time of day as `i64` (microseconds / nanoseconds; Arrow `Time64`).
    Time64 = 0x0065,
    /// Instant as `i32` count since the epoch (narrow `Timestamp`).
    Ts32 = 0x0068,
    /// Instant as `i64` count since the epoch (Arrow `Timestamp`).
    Ts64 = 0x0069,
    /// Instant as a 96-bit count since the epoch (wide `Timestamp`).
    Ts96 = 0x006A,
    /// Elapsed span as an `i32` count (narrow `Duration`).
    Duration32 = 0x006C,
    /// Elapsed span as an `i64` count (Arrow `Duration`).
    Duration64 = 0x006D,

    /// Variable-length opaque binary (`i32` offsets).
    Binary = 0x0100,
    /// Variable-length opaque binary (`i64` offsets).
    LargeBinary = 0x0101,
    /// Variable-length UTF-8 string (`i32` offsets).
    Utf8 = 0x0108,
    /// Variable-length UTF-8 string (`i64` offsets).
    LargeUtf8 = 0x0109,

    /// A **struct** — an ordered, named set of child fields (Arrow `Struct`). Its children live
    /// on the descriptor / field, not on the id.
    Struct = 0x0200,
    /// A **list** — a variable-size sequence of a single element type (Arrow `List`, `i32`
    /// offsets). The element type lives on the descriptor / field.
    List = 0x0210,
    /// A **map** — an unordered set of `key → value` entries (Arrow `Map`). The key/value types
    /// live on the descriptor / field.
    Map = 0x0220,
}

impl DataTypeId {
    /// The reserved field-metadata key under which a field records its **exact** logical type
    /// (its [`name`](DataTypeId::name)), so a lossy Arrow mapping — e.g. `u96`/`i96` and
    /// `FixedUtf8` all becoming `FixedSizeBinary` — round-trips back to the precise type. Arrow
    /// carries unknown metadata keys through IPC/Parquet, so the discriminator survives.
    pub const METADATA_KEY: &'static str = "yggdryl.logical_type";

    /// The field-metadata key under which a **decimal** field records its `precision`, so an erased
    /// [`Field`](crate::io::fixed::Field) round-trips the precision that neither its
    /// [`type_id`](DataTypeId) nor its byte width captures (the width fixes only the coefficient
    /// integer, not the `Decimal(precision, scale)` parameters). Kept a short, shared notion — only
    /// the type discriminator ([`METADATA_KEY`](DataTypeId::METADATA_KEY)) is `yggdryl`-namespaced.
    pub const PRECISION_METADATA_KEY: &'static str = "precision";

    /// The field-metadata key under which a **decimal** field records its `scale` (see
    /// [`PRECISION_METADATA_KEY`](DataTypeId::PRECISION_METADATA_KEY)).
    pub const SCALE_METADATA_KEY: &'static str = "scale";

    /// The field-metadata key under which a **temporal** field records its
    /// [`TimeUnit`](crate::io::fixed::temporal::TimeUnit) name — the resolution the id + byte width
    /// alone do not pin (a `Time32` may be seconds or milliseconds, a timestamp any fixed unit).
    pub const TIME_UNIT_METADATA_KEY: &'static str = "unit";

    /// The field-metadata key under which a **timestamp** field records its timezone name (empty
    /// for naive), so a zoned timestamp round-trips its zone.
    pub const TIMEZONE_METADATA_KEY: &'static str = "timezone";

    /// The raw `u16` discriminant.
    pub const fn as_u16(self) -> u16 {
        self as u16
    }

    /// `lo <= self <= hi` — the one range primitive every predicate reduces to.
    const fn in_range(self, lo: u16, hi: u16) -> bool {
        let value = self as u16;
        value >= lo && value <= hi
    }

    /// Whether this is the null type.
    pub const fn is_null(self) -> bool {
        (self as u16) == 0x0000
    }

    /// Whether the type is an **unsigned** integer.
    pub const fn is_unsigned_integer(self) -> bool {
        self.in_range(0x0010, 0x001F)
    }

    /// Whether the type is a **signed** integer.
    pub const fn is_signed_integer(self) -> bool {
        self.in_range(0x0020, 0x002F)
    }

    /// Whether the type is any integer (signed or unsigned).
    pub const fn is_integer(self) -> bool {
        self.in_range(0x0010, 0x002F)
    }

    /// Whether the type is a float.
    pub const fn is_floating(self) -> bool {
        self.in_range(0x0030, 0x003F)
    }

    /// Whether the type is a **scaled decimal** (`D32`…`D256`).
    pub const fn is_decimal(self) -> bool {
        self.in_range(0x0050, 0x005F)
    }

    /// Whether the type is a **temporal** value — a date, time, timestamp, or duration.
    pub const fn is_temporal(self) -> bool {
        self.in_range(0x0060, 0x007F)
    }

    /// Whether the type is any number (integer, float, or decimal). Decimals sit outside the
    /// contiguous integer/float band (the fixed-size byte types occupy the gap), so this is two
    /// ranges rather than one.
    pub const fn is_numeric(self) -> bool {
        self.in_range(0x0010, 0x003F) || self.is_decimal()
    }

    /// Whether the type is a **signed** number (signed integer, float, or decimal — every
    /// decimal is signed).
    pub const fn is_signed(self) -> bool {
        self.in_range(0x0020, 0x003F) || self.is_decimal()
    }

    /// The maximum precision (significant decimal digits) a value of this decimal id can hold —
    /// `9`/`18`/`38`/`76` for `D32`/`D64`/`D128`/`D256` — or `None` for a non-decimal id. Matches
    /// Arrow's `DECIMAL{32,64,128,256}_MAX_PRECISION`.
    pub const fn decimal_max_precision(self) -> Option<u8> {
        Some(match self {
            Self::D32 => 9,
            Self::D64 => 18,
            Self::D128 => 38,
            Self::D256 => 76,
            _ => return None,
        })
    }

    /// Whether values have a fixed byte width (numbers, and the fixed-size byte types).
    pub const fn is_fixed_width(self) -> bool {
        self.in_range(0x0001, 0x00FF)
    }

    /// Whether values are variable-length (`Binary`/`Utf8` and their `Large` forms).
    pub const fn is_variable_length(self) -> bool {
        self.in_range(0x0100, 0x01FF)
    }

    /// Whether the type is opaque binary — fixed-size or variable-length.
    pub const fn is_binary(self) -> bool {
        self.in_range(0x0040, 0x0047) || self.in_range(0x0100, 0x0107)
    }

    /// Whether the type is a UTF-8 string — fixed-size or variable-length.
    pub const fn is_utf8(self) -> bool {
        self.in_range(0x0048, 0x004F) || self.in_range(0x0108, 0x010F)
    }

    /// Whether the type is a **nested / composite** type — a struct, list, or map. A nested type
    /// is neither [`is_fixed_width`](DataTypeId::is_fixed_width) nor
    /// [`is_variable_length`](DataTypeId::is_variable_length) (those bands are the *leaf* types); it
    /// carries its child field(s) on the descriptor, not on the id.
    pub const fn is_nested(self) -> bool {
        self.in_range(0x0200, 0x02FF)
    }

    /// Whether the type is a **struct** (an ordered, named set of child fields).
    pub const fn is_struct(self) -> bool {
        self.in_range(0x0200, 0x020F)
    }

    /// Whether the type is a **list** (a variable-size sequence of one element type).
    pub const fn is_list(self) -> bool {
        self.in_range(0x0210, 0x021F)
    }

    /// Whether the type is a **map** (a set of `key → value` entries).
    pub const fn is_map(self) -> bool {
        self.in_range(0x0220, 0x022F)
    }

    /// The coarse [`DataTypeCategory`] bucket this id falls in.
    pub const fn category(self) -> DataTypeCategory {
        if self.is_unsigned_integer() {
            DataTypeCategory::UnsignedInteger
        } else if self.is_signed_integer() {
            DataTypeCategory::SignedInteger
        } else if self.is_floating() {
            DataTypeCategory::Float
        } else if self.is_decimal() {
            DataTypeCategory::Decimal
        } else if self.is_temporal() {
            DataTypeCategory::Temporal
        } else if self.is_utf8() {
            DataTypeCategory::Utf8
        } else if self.is_binary() {
            DataTypeCategory::Binary
        } else if self.is_nested() {
            DataTypeCategory::Nested
        } else {
            DataTypeCategory::Null
        }
    }

    /// Decodes a `u16` discriminant, returning `None` for an unknown / reserved value (never a
    /// transmute — a reserved gap value is not a valid `DataTypeId`).
    pub const fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0x0000 => Self::Null,
            0x0010 => Self::U8,
            0x0011 => Self::U16,
            0x0012 => Self::U32,
            0x0013 => Self::U64,
            0x0014 => Self::U96,
            0x0015 => Self::U128,
            0x0016 => Self::U256,
            0x0020 => Self::I8,
            0x0021 => Self::I16,
            0x0022 => Self::I32,
            0x0023 => Self::I64,
            0x0024 => Self::I96,
            0x0025 => Self::I128,
            0x0026 => Self::I256,
            0x0030 => Self::F16,
            0x0031 => Self::F32,
            0x0032 => Self::F64,
            0x0040 => Self::FixedBinary,
            0x0048 => Self::FixedUtf8,
            0x0050 => Self::D32,
            0x0051 => Self::D64,
            0x0052 => Self::D128,
            0x0053 => Self::D256,
            0x0060 => Self::Date32,
            0x0061 => Self::Date64,
            0x0064 => Self::Time32,
            0x0065 => Self::Time64,
            0x0068 => Self::Ts32,
            0x0069 => Self::Ts64,
            0x006A => Self::Ts96,
            0x006C => Self::Duration32,
            0x006D => Self::Duration64,
            0x0100 => Self::Binary,
            0x0101 => Self::LargeBinary,
            0x0108 => Self::Utf8,
            0x0109 => Self::LargeUtf8,
            0x0200 => Self::Struct,
            0x0210 => Self::List,
            0x0220 => Self::Map,
            _ => return None,
        })
    }

    /// The stable, lower-case **canonical name** of this type (`"u8"`, `"i256"`, `"fixed_utf8"`,
    /// `"large_binary"`, …) — the same names the concrete types report, and the value stored in
    /// field metadata to round-trip the exact logical type through Arrow. The inverse of
    /// [`from_name`](DataTypeId::from_name).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U96 => "u96",
            Self::U128 => "u128",
            Self::U256 => "u256",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I96 => "i96",
            Self::I128 => "i128",
            Self::I256 => "i256",
            Self::F16 => "f16",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::FixedBinary => "fixed_binary",
            Self::FixedUtf8 => "fixed_utf8",
            Self::D32 => "d32",
            Self::D64 => "d64",
            Self::D128 => "d128",
            Self::D256 => "d256",
            Self::Date32 => "date32",
            Self::Date64 => "date64",
            Self::Time32 => "time32",
            Self::Time64 => "time64",
            Self::Ts32 => "ts32",
            Self::Ts64 => "ts64",
            Self::Ts96 => "ts96",
            Self::Duration32 => "duration32",
            Self::Duration64 => "duration64",
            Self::Binary => "binary",
            Self::LargeBinary => "large_binary",
            Self::Utf8 => "utf8",
            Self::LargeUtf8 => "large_utf8",
            Self::Struct => "struct",
            Self::List => "list",
            Self::Map => "map",
        }
    }

    /// The **intrinsic** byte width of a value's fixed portion for this id, or `None` when the
    /// width is not fixed by the id alone (the runtime-`N` [`FixedBinary`](DataTypeId::FixedBinary)
    /// / [`FixedUtf8`](DataTypeId::FixedUtf8)). It is the numeric primitive's width, and for the
    /// variable-length types the width of one *offset* (`4` for `i32`, `8` for the `Large` `i64`
    /// forms). Agrees with the concrete descriptors' [`byte_width`](crate::io::DataType::byte_width).
    pub const fn fixed_byte_width(self) -> Option<usize> {
        Some(match self {
            Self::Null => 0,
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 | Self::F16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
            Self::U96 | Self::I96 => 12,
            Self::U128 | Self::I128 => 16,
            Self::U256 | Self::I256 => 32,
            Self::D32 => 4,   // i32 coefficient
            Self::D64 => 8,   // i64 coefficient
            Self::D128 => 16, // i128 coefficient
            Self::D256 => 32, // 256-bit coefficient
            Self::Date32 | Self::Time32 | Self::Ts32 | Self::Duration32 => 4,
            Self::Date64 | Self::Time64 | Self::Ts64 | Self::Duration64 => 8,
            Self::Ts96 => 12,
            Self::Binary | Self::Utf8 => 4,           // i32 offsets
            Self::LargeBinary | Self::LargeUtf8 => 8, // i64 offsets
            Self::FixedBinary | Self::FixedUtf8 => return None, // width is the runtime N
            // Nested types have no fixed byte width — their shape lives on the descriptor's
            // children, not the id.
            Self::Struct | Self::List | Self::Map => return None,
        })
    }

    /// The id for a canonical [`name`](DataTypeId::name), or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "null" => Self::Null,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "u96" => Self::U96,
            "u128" => Self::U128,
            "u256" => Self::U256,
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "i96" => Self::I96,
            "i128" => Self::I128,
            "i256" => Self::I256,
            "f16" => Self::F16,
            "f32" => Self::F32,
            "f64" => Self::F64,
            "fixed_binary" => Self::FixedBinary,
            "fixed_utf8" => Self::FixedUtf8,
            "d32" => Self::D32,
            "d64" => Self::D64,
            "d128" => Self::D128,
            "d256" => Self::D256,
            "date32" => Self::Date32,
            "date64" => Self::Date64,
            "time32" => Self::Time32,
            "time64" => Self::Time64,
            "ts32" => Self::Ts32,
            "ts64" => Self::Ts64,
            "ts96" => Self::Ts96,
            "duration32" => Self::Duration32,
            "duration64" => Self::Duration64,
            "binary" => Self::Binary,
            "large_binary" => Self::LargeBinary,
            "utf8" => Self::Utf8,
            "large_utf8" => Self::LargeUtf8,
            "struct" => Self::Struct,
            "list" => Self::List,
            "map" => Self::Map,
            _ => return None,
        })
    }
}

/// The centralized, **total** Arrow data-type mapping (feature `arrow`) — the single place the
/// type space is mapped to/from Arrow, so the erased [`Field`](crate::io::fixed::Field) and every
/// typed field share one definition.
#[cfg(feature = "arrow")]
impl DataTypeId {
    /// The matching **or closest** Arrow [`DataType`](arrow_schema::DataType) for a value of this
    /// id and `byte_width` (the width matters only for the fixed-size byte types, which map to
    /// `FixedSizeBinary(N)`). Total: exact where Arrow has the primitive, else the closest
    /// representation — `Decimal128`/`Decimal256` for the signed 128/256-bit ints,
    /// `FixedSizeBinary(N)` for the widths Arrow cannot model. Lossy mappings are disambiguated
    /// on the way back by the field's metadata (see [`from_arrow`](DataTypeId::from_arrow)).
    pub fn to_arrow(self, byte_width: usize) -> arrow_schema::DataType {
        use arrow_schema::DataType as A;
        match self {
            Self::Null => A::Null,
            Self::U8 => A::UInt8,
            Self::U16 => A::UInt16,
            Self::U32 => A::UInt32,
            Self::U64 => A::UInt64,
            Self::I8 => A::Int8,
            Self::I16 => A::Int16,
            Self::I32 => A::Int32,
            Self::I64 => A::Int64,
            Self::F16 => A::Float16,
            Self::F32 => A::Float32,
            Self::F64 => A::Float64,
            Self::I128 => A::Decimal128(38, 0),
            Self::I256 => A::Decimal256(76, 0),
            // The decimals default to a scale-0 max-precision `Decimal` when only the id is known
            // (the erased path); the precise `Decimal(precision, scale)` comes from
            // [`to_arrow_decimal`](DataTypeId::to_arrow_decimal), which the typed descriptor and
            // the metadata-carrying field use.
            Self::D32 => A::Decimal32(9, 0),
            Self::D64 => A::Decimal64(18, 0),
            Self::D128 => A::Decimal128(38, 0),
            Self::D256 => A::Decimal256(76, 0),
            // Temporal defaults when only the id is known (the erased path); the precise unit / tz
            // ride the reserved metadata keys (a `Ts32`/`Ts96`/`Duration32` widens or
            // maps to `FixedSizeBinary`, recovered via the logical-type tag).
            Self::Date32 => A::Date32,
            Self::Date64 => A::Date64,
            Self::Time32 => A::Time32(arrow_schema::TimeUnit::Millisecond),
            Self::Time64 => A::Time64(arrow_schema::TimeUnit::Nanosecond),
            Self::Ts32 | Self::Ts64 => A::Timestamp(arrow_schema::TimeUnit::Nanosecond, None),
            Self::Ts96 => A::FixedSizeBinary(12),
            Self::Duration32 | Self::Duration64 => A::Duration(arrow_schema::TimeUnit::Nanosecond),
            Self::U128 => A::FixedSizeBinary(16),
            Self::U96 | Self::I96 => A::FixedSizeBinary(12),
            Self::U256 => A::FixedSizeBinary(32),
            Self::FixedBinary | Self::FixedUtf8 => A::FixedSizeBinary(byte_width as i32),
            Self::Binary => A::Binary,
            Self::LargeBinary => A::LargeBinary,
            Self::Utf8 => A::Utf8,
            Self::LargeUtf8 => A::LargeUtf8,
            // DESIGN: nested types cannot be built from the id + width alone — their Arrow type
            // needs the child field(s), which live on the typed descriptor / erased nested field.
            // These structural *shells* keep the id-level mapping total; the real, recursive
            // mapping is `StructField`/`ListField`/`MapField::to_arrow`. Never on the real path.
            Self::Struct => A::Struct(arrow_schema::Fields::empty()),
            Self::List => A::List(std::sync::Arc::new(arrow_schema::Field::new(
                "item",
                A::Null,
                true,
            ))),
            Self::Map => A::Map(
                std::sync::Arc::new(arrow_schema::Field::new(
                    "entries",
                    A::Struct(arrow_schema::Fields::from(vec![
                        arrow_schema::Field::new("keys", A::Null, false),
                        arrow_schema::Field::new("values", A::Null, true),
                    ])),
                    false,
                )),
                false,
            ),
        }
    }

    /// The Arrow `Decimal{32,64,128,256}(precision, scale)` for a **decimal** id — the exact
    /// mapping the typed [`DecimalType`](crate::io::fixed::DecimalType) and the metadata-carrying
    /// [`Field`](crate::io::fixed::Field) use, since `Decimal` needs the `(precision, scale)` that
    /// the id + byte width alone cannot supply. Returns `None` for a non-decimal id.
    pub fn to_arrow_decimal(self, precision: u8, scale: i8) -> Option<arrow_schema::DataType> {
        use arrow_schema::DataType as A;
        Some(match self {
            Self::D32 => A::Decimal32(precision, scale),
            Self::D64 => A::Decimal64(precision, scale),
            Self::D128 => A::Decimal128(precision, scale),
            Self::D256 => A::Decimal256(precision, scale),
            _ => return None,
        })
    }

    /// The `(precision, scale)` of an Arrow `Decimal{32,64,128,256}` data type, or `None` for any
    /// other type — the parameters the erased [`Field`](crate::io::fixed::Field) carries across a
    /// decimal round-trip.
    pub fn arrow_decimal_params(data_type: &arrow_schema::DataType) -> Option<(u8, i8)> {
        use arrow_schema::DataType as A;
        match data_type {
            A::Decimal32(p, s) | A::Decimal64(p, s) | A::Decimal128(p, s) | A::Decimal256(p, s) => {
                Some((*p, *s))
            }
            _ => None,
        }
    }

    /// The **column-level** Arrow [`DataType`](arrow_schema::DataType) for a **temporal** id with
    /// resolution `unit` and zone `tz` — the type the typed
    /// [`TemporalType`](crate::io::fixed::TemporalType) and the metadata-carrying `Field` /
    /// `TemporalSerie` array use, since a temporal type needs the `(unit, tz)` that the id + byte
    /// width alone cannot supply. Total over the temporal band:
    ///
    /// - `Date32` → `Date32`, `Date64` → `Date64` (Arrow fixes their resolution — `unit` is ignored);
    /// - `Time32` → `Time32(unit)`, `Time64` → `Time64(unit)`;
    /// - `Ts32` / `Ts64` → `Timestamp(unit, tz)` (the narrow `Ts32` **widens** to the same `i64`
    ///   `Timestamp`; the field's logical-type tag recovers the narrow id on import);
    /// - `Ts96` → `FixedSizeBinary(12)` — Arrow has no 96-bit temporal type, so its unit/tz ride the
    ///   field metadata (a **lossy** type mapping);
    /// - `Duration32` / `Duration64` → `Duration(unit)`.
    ///
    /// Returns `None` for a non-temporal id, or for a `unit` Arrow cannot represent (`Minute`…`Year`,
    /// whose [`TimeUnit::to_arrow`](crate::io::fixed::temporal::TimeUnit) is `None`) on the
    /// unit-bearing types.
    #[cfg(feature = "arrow")]
    pub fn to_arrow_temporal(self, unit: TimeUnit, tz: Tz) -> Option<arrow_schema::DataType> {
        use arrow_schema::DataType as A;
        let zone = |tz: Tz| {
            if tz.is_naive() {
                None
            } else {
                Some(std::sync::Arc::<str>::from(tz.name()))
            }
        };
        Some(match self {
            Self::Date32 => A::Date32,
            Self::Date64 => A::Date64,
            // Time32/Time64 admit only their own unit sub-domain — reject an out-of-domain unit
            // rather than emit the spec-invalid `Time32(Nanosecond)` / `Time64(Second)`.
            Self::Time32 if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) => {
                A::Time32(unit.to_arrow()?)
            }
            Self::Time64 if matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond) => {
                A::Time64(unit.to_arrow()?)
            }
            Self::Ts32 | Self::Ts64 => A::Timestamp(unit.to_arrow()?, zone(tz)),
            Self::Ts96 => A::FixedSizeBinary(12),
            Self::Duration32 | Self::Duration64 => A::Duration(unit.to_arrow()?),
            _ => return None,
        })
    }

    /// The `(unit, tz)` an Arrow temporal data type denotes, or `None` for any other type (including
    /// the `ts96` `FixedSizeBinary` form, whose axes ride the field metadata) — the inverse the
    /// erased [`Field`](crate::io::fixed::Field) and `TemporalSerie` use to recover a temporal
    /// column's resolution / zone.
    #[cfg(feature = "arrow")]
    pub fn arrow_temporal_params(data_type: &arrow_schema::DataType) -> Option<(TimeUnit, Tz)> {
        use arrow_schema::DataType as A;
        Some(match data_type {
            A::Date32 => (TimeUnit::Day, Tz::NAIVE),
            A::Date64 => (TimeUnit::Millisecond, Tz::NAIVE),
            A::Time32(u) | A::Time64(u) => (TimeUnit::from_arrow(*u), Tz::NAIVE),
            A::Timestamp(u, zone) => {
                let tz = match zone {
                    Some(name) => Tz::parse(name).unwrap_or(Tz::UTC),
                    None => Tz::NAIVE,
                };
                (TimeUnit::from_arrow(*u), tz)
            }
            A::Duration(u) => (TimeUnit::from_arrow(*u), Tz::NAIVE),
            _ => return None,
        })
    }

    /// The `(id, byte_width)` a raw Arrow [`DataType`](arrow_schema::DataType) maps to in this
    /// crate's scheme, or `None` for a type this crate does not model. **Best-effort**: the
    /// ambiguous `FixedSizeBinary(N)` defaults to [`FixedBinary`](DataTypeId::FixedBinary) (it
    /// could equally be a wide integer or `FixedUtf8`) — field metadata refines it. The
    /// (near-)inverse of [`to_arrow`](DataTypeId::to_arrow).
    pub fn from_arrow(data_type: &arrow_schema::DataType) -> Option<(Self, usize)> {
        use arrow_schema::DataType as A;
        Some(match data_type {
            A::Null => (Self::Null, 0),
            A::UInt8 => (Self::U8, 1),
            A::UInt16 => (Self::U16, 2),
            A::UInt32 => (Self::U32, 4),
            A::UInt64 => (Self::U64, 8),
            A::Int8 => (Self::I8, 1),
            A::Int16 => (Self::I16, 2),
            A::Int32 => (Self::I32, 4),
            A::Int64 => (Self::I64, 8),
            A::Float16 => (Self::F16, 2),
            A::Float32 => (Self::F32, 4),
            A::Float64 => (Self::F64, 8),
            // The narrow decimals are unambiguous — nothing else maps to `Decimal32`/`Decimal64`.
            A::Decimal32(_, _) => (Self::D32, 4),
            A::Decimal64(_, _) => (Self::D64, 8),
            // `Decimal128`/`Decimal256` default to the wide *integers* (their closest-rep source),
            // so a `D128`/`D256` field carries the `yggdryl.logical_type` tag to be recovered as a
            // decimal (see [`Field::from_arrow`](crate::io::fixed::Field::from_arrow)); the
            // precision/scale ride the reserved metadata keys.
            A::Decimal128(_, _) => (Self::I128, 16),
            A::Decimal256(_, _) => (Self::I256, 32),
            // Temporal: the unit / tz ride the field metadata; the narrow `Ts32` /
            // `Ts96` / `Duration32` recover via the logical-type tag.
            A::Date32 => (Self::Date32, 4),
            A::Date64 => (Self::Date64, 8),
            A::Time32(_) => (Self::Time32, 4),
            A::Time64(_) => (Self::Time64, 8),
            A::Timestamp(_, _) => (Self::Ts64, 8),
            A::Duration(_) => (Self::Duration64, 8),
            A::FixedSizeBinary(n) => (Self::FixedBinary, (*n).max(0) as usize),
            A::Binary => (Self::Binary, 4),
            A::LargeBinary => (Self::LargeBinary, 8),
            A::Utf8 => (Self::Utf8, 4),
            A::LargeUtf8 => (Self::LargeUtf8, 8),
            _ => return None,
        })
    }
}

// The category ranges above are load-bearing: `is_integer` / `is_numeric` / `is_signed` are
// single ranges *only* because the blocks are laid out unsigned < signed < float, contiguously.
// Lock that adjacency so a future reordering edit fails to compile instead of silently
// mis-categorizing.
const _: () = {
    assert!(DataTypeId::U8 as u16 == 0x0010);
    assert!(DataTypeId::I8 as u16 == 0x0020);
    assert!(DataTypeId::F16 as u16 == 0x0030);
    // unsigned block ends below the signed block, which ends below the float block.
    assert!((DataTypeId::U256 as u16) < (DataTypeId::I8 as u16));
    assert!((DataTypeId::I256 as u16) < (DataTypeId::F16 as u16));
    // Every fixed-width id is below the variable-length band.
    assert!((DataTypeId::FixedUtf8 as u16) < (DataTypeId::Binary as u16));
    assert!(DataTypeId::Binary as u16 == 0x0100);
    // The decimal block is contiguous, above the fixed-size byte types and still fixed-width, so
    // `is_decimal` is one bounded range and the decimals never leak into `is_binary`/`is_utf8`.
    assert!(DataTypeId::D32 as u16 == 0x0050);
    assert!((DataTypeId::FixedUtf8 as u16) < (DataTypeId::D32 as u16));
    assert!((DataTypeId::D256 as u16) < (DataTypeId::Binary as u16));
    assert!(DataTypeId::D32.is_decimal() && DataTypeId::D256.is_decimal());
    assert!(DataTypeId::D128.is_numeric() && DataTypeId::D128.is_signed());
    assert!(DataTypeId::D128.is_fixed_width());
    // The temporal block is contiguous, above the decimals and below the variable band; it is
    // fixed-width but NOT numeric (a date is not a number).
    assert!(DataTypeId::Date32 as u16 == 0x0060);
    assert!((DataTypeId::D256 as u16) < (DataTypeId::Date32 as u16));
    assert!((DataTypeId::Duration64 as u16) < (DataTypeId::Binary as u16));
    assert!(DataTypeId::Date32.is_temporal() && DataTypeId::Duration64.is_temporal());
    assert!(DataTypeId::Ts64.is_fixed_width() && !DataTypeId::Ts64.is_numeric());
    assert!(!DataTypeId::Date32.is_binary() && !DataTypeId::Date32.is_utf8());
    // The nested band sits in its own reserved space *above* every leaf type (fixed and
    // variable), so `is_nested` never overlaps a leaf predicate and the leaf bands stay bounded.
    assert!(DataTypeId::Struct as u16 == 0x0200);
    assert!(DataTypeId::List as u16 == 0x0210);
    assert!(DataTypeId::Map as u16 == 0x0220);
    assert!((DataTypeId::LargeUtf8 as u16) < (DataTypeId::Struct as u16));
    assert!(
        DataTypeId::Struct.is_nested()
            && DataTypeId::List.is_nested()
            && DataTypeId::Map.is_nested()
    );
    assert!(
        DataTypeId::Struct.is_struct() && DataTypeId::List.is_list() && DataTypeId::Map.is_map()
    );
    // A nested type is neither fixed-width nor variable-length, and is not any leaf category.
    assert!(!DataTypeId::Struct.is_fixed_width() && !DataTypeId::Struct.is_variable_length());
    assert!(
        !DataTypeId::List.is_numeric()
            && !DataTypeId::Map.is_binary()
            && !DataTypeId::Map.is_utf8()
    );
};
