//! [`PrimitiveType`] — a runtime dtype tag and the dtype-keyed converter facade.

use crate::{BytesConverter, CastConverter, ConvertError, Converter, StringConverter};

/// The ten native primitive element types, as a runtime tag.
///
/// The typed converters ([`CastConverter`], [`StringConverter`], …) fix their element
/// types at compile time, which the FFI cannot hold. `PrimitiveType` recovers the
/// choice at runtime: it names a primitive with a string (`"i32"`), reports its
/// [`width`](PrimitiveType::width), and drives the dtype-keyed facade
/// ([`cast_bytes`](PrimitiveType::cast_bytes) /
/// [`parse_bytes`](PrimitiveType::parse_bytes) /
/// [`format_bytes`](PrimitiveType::format_bytes)) that the Python and Node bindings
/// expose as `yggdryl.converter`.
///
/// ```
/// use yggdryl_core::PrimitiveType;
///
/// let i32 = PrimitiveType::from_name("i32").unwrap();
/// assert_eq!(i32.width(), 4);
///
/// // Widen little-endian i32 bytes to i64 bytes.
/// let wide = i32.cast_bytes(PrimitiveType::I64, &7_i32.to_le_bytes()).unwrap();
/// assert_eq!(wide, 7_i64.to_le_bytes());
///
/// // Flexible parse, then render back.
/// let bytes = i32.parse_bytes("0x2A").unwrap();
/// assert_eq!(bytes, 42_i32.to_le_bytes());
/// assert_eq!(i32.format_bytes(&bytes).unwrap(), "42");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PrimitiveType {
    /// Signed 8-bit integer.
    I8,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer.
    I32,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 8-bit integer.
    U8,
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Unsigned 64-bit integer.
    U64,
    /// 32-bit IEEE-754 float.
    F32,
    /// 64-bit IEEE-754 float.
    F64,
}

impl PrimitiveType {
    /// The accepted dtype names, for error messages.
    pub const EXPECTED: &'static str = "i8, i16, i32, i64, u8, u16, u32, u64, f32, f64";

    /// Flexibly parses `text` into one value of this type, returning its little-endian
    /// bytes (see [`StringConverter`]) — the `&str` convenience over
    /// [`string_convert_bytes`](PrimitiveType::string_convert_bytes).
    pub fn parse_bytes(self, text: &str) -> Result<Vec<u8>, ConvertError> {
        self.string_convert_bytes(text.as_bytes())
    }

    /// Renders one value of this type (from its little-endian `bytes`) to its string
    /// form (see [`StringConverter`]) — the `String` convenience over
    /// [`string_invert_bytes`](PrimitiveType::string_invert_bytes).
    pub fn format_bytes(self, bytes: &[u8]) -> Result<String, ConvertError> {
        let out = self.string_invert_bytes(bytes)?;
        Ok(String::from_utf8(out).expect("Display output is valid UTF-8"))
    }
}

/// Generates the [`PrimitiveType`] facade from the `(variant, type, name)` list: the
/// name/width lookups, `from_name`, and the dtype-keyed byte converters. The cast
/// dispatch is a full `from × to` matrix, so the type list is captured once as a `tt`
/// and threaded into the inner match to avoid a metavariable-depth clash.
macro_rules! primitive_type {
    ($(($pt:ident, $ty:ty, $name:literal)),+ $(,)?) => {
        impl PrimitiveType {
            /// Every primitive, in declaration order.
            pub const ALL: &'static [PrimitiveType] = &[$(PrimitiveType::$pt),+];

            /// The dtype name, e.g. `"i32"`.
            pub const fn name(self) -> &'static str {
                match self { $(PrimitiveType::$pt => $name),+ }
            }

            /// The element width in bytes.
            pub const fn width(self) -> usize {
                match self { $(PrimitiveType::$pt => core::mem::size_of::<$ty>()),+ }
            }

            /// Resolves a dtype name, or a guided [`ConvertError::UnknownType`].
            pub fn from_name(name: &str) -> Result<Self, ConvertError> {
                match name {
                    $($name => Ok(PrimitiveType::$pt),)+
                    _ => Err(ConvertError::UnknownType {
                        name: name.to_string(),
                        expected: Self::EXPECTED,
                    }),
                }
            }

            /// Casts packed little-endian `bytes` of this type to `to`'s little-endian
            /// bytes (C-style `as`), for every source/target pair.
            pub fn cast_bytes(self, to: PrimitiveType, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
                primitive_type!(@cast self, to, bytes, [$(($pt, $ty)),+], [$(($pt, $ty)),+])
            }

            /// Parses UTF-8 text `bytes` into one value of this type (flexible formats),
            /// returning its little-endian bytes — the [`StringConverter`] forward
            /// byte-array conversion.
            pub fn string_convert_bytes(self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
                match self {
                    $(PrimitiveType::$pt =>
                        StringConverter::<$ty>::new().convert_byte_array(bytes),)+
                }
            }

            /// Renders one value of this type (from its little-endian `bytes`) to its
            /// UTF-8 text bytes — the [`StringConverter`] inverse byte-array conversion.
            pub fn string_invert_bytes(self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
                match self {
                    $(PrimitiveType::$pt =>
                        StringConverter::<$ty>::new().invert_byte_array(bytes),)+
                }
            }

            /// Validates that `bytes` is a whole number of this type's values and
            /// returns them unchanged — the [`BytesConverter`] byte-array conversion
            /// (identical forward and inverse).
            pub fn bytes_convert_bytes(self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
                match self {
                    $(PrimitiveType::$pt =>
                        BytesConverter::<$ty>::new().convert_byte_array(bytes),)+
                }
            }
        }
    };
    // Outer arm: iterate sources, threading the captured target list `$all`.
    (@cast $from:expr, $to:expr, $bytes:expr, [$(($spt:ident, $sty:ty)),+], $all:tt) => {
        match $from {
            $( PrimitiveType::$spt => primitive_type!(@cast_dst $sty, $to, $bytes, $all), )+
        }
    };
    // Inner arm: for a fixed source `$src`, iterate targets.
    (@cast_dst $src:ty, $to:expr, $bytes:expr, [$(($dpt:ident, $dty:ty)),+]) => {
        match $to {
            $( PrimitiveType::$dpt =>
                CastConverter::<$src, $dty>::new().convert_byte_array($bytes), )+
        }
    };
}

primitive_type! {
    (I8, i8, "i8"),
    (I16, i16, "i16"),
    (I32, i32, "i32"),
    (I64, i64, "i64"),
    (U8, u8, "u8"),
    (U16, u16, "u16"),
    (U32, u32, "u32"),
    (U64, u64, "u64"),
    (F32, f32, "f32"),
    (F64, f64, "f64"),
}
