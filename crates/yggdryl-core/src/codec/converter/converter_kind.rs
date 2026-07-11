//! [`ConverterKind`] — a runtime tag naming a converter family, and the byte-array
//! `convert` / `invert` facade over it.

use crate::{ConvertError, Converter, PrimitiveType, Utf8Converter};

/// The converter families, as a runtime tag — the "overall" converter selector.
///
/// The concrete converters ([`CastConverter`](crate::CastConverter),
/// [`StringConverter`](crate::StringConverter), …) fix their element types at compile
/// time, which the FFI cannot hold. `ConverterKind` recovers the **choice of
/// converter** at runtime, the way [`PrimitiveType`] recovers the choice of element
/// type: it names a family with a string (`"cast"`) and drives the whole-buffer facade
/// ([`convert_bytes`](ConverterKind::convert_bytes) /
/// [`invert_bytes`](ConverterKind::invert_bytes)) that the Python and Node bindings
/// expose as `yggdryl.converter.convert_bytes` / `invert_bytes` — the general convert
/// over a whole byte array, and its exact inverse.
///
/// Each family takes the dtype arguments it needs: [`Cast`](ConverterKind::Cast) a
/// source and a target dtype, [`String`](ConverterKind::String) and
/// [`Bytes`](ConverterKind::Bytes) a single dtype, [`Utf8`](ConverterKind::Utf8) none.
/// A required dtype left out yields a guided [`ConvertError::MissingDtype`].
///
/// ```
/// use yggdryl_core::{ConverterKind, PrimitiveType};
///
/// // Cast: widen a whole buffer of i32 bytes to i64, then invert back to i32.
/// let cast = ConverterKind::from_name("cast").unwrap();
/// let i32 = Some(PrimitiveType::I32);
/// let i64 = Some(PrimitiveType::I64);
/// let wide = cast.convert_bytes(&7_i32.to_le_bytes(), i32, i64).unwrap();
/// assert_eq!(wide, 7_i64.to_le_bytes());
/// assert_eq!(cast.invert_bytes(&wide, i32, i64).unwrap(), 7_i32.to_le_bytes());
///
/// // String: UTF-8 text bytes to i32 bytes, and invert back to text.
/// let string = ConverterKind::from_name("string").unwrap();
/// let bytes = string.convert_bytes(b"42", i32, None).unwrap();
/// assert_eq!(bytes, 42_i32.to_le_bytes());
/// assert_eq!(string.invert_bytes(&bytes, i32, None).unwrap(), b"42");
///
/// // A missing dtype is a guided error.
/// let err = cast.convert_bytes(&[], i32, None).unwrap_err();
/// assert!(err.to_string().contains("needs a to dtype"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConverterKind {
    /// Numeric C-style cast between two primitive dtypes (needs a `from` and a `to`).
    Cast,
    /// Flexible string parse/render for one dtype (needs a `dtype`).
    String,
    /// A primitive value to/from its little-endian bytes for one dtype (needs a
    /// `dtype`).
    Bytes,
    /// A string to/from its validated UTF-8 bytes (no dtype).
    Utf8,
}

impl ConverterKind {
    /// The accepted converter names, for error messages.
    pub const EXPECTED: &'static str = "cast, string, bytes, utf8";

    /// Every converter kind, in declaration order.
    pub const ALL: &'static [ConverterKind] = &[
        ConverterKind::Cast,
        ConverterKind::String,
        ConverterKind::Bytes,
        ConverterKind::Utf8,
    ];

    /// The converter name, e.g. `"cast"`.
    pub const fn name(self) -> &'static str {
        match self {
            ConverterKind::Cast => "cast",
            ConverterKind::String => "string",
            ConverterKind::Bytes => "bytes",
            ConverterKind::Utf8 => "utf8",
        }
    }

    /// Resolves a converter name, or a guided [`ConvertError::UnknownConverter`].
    pub fn from_name(name: &str) -> Result<Self, ConvertError> {
        match name {
            "cast" => Ok(ConverterKind::Cast),
            "string" => Ok(ConverterKind::String),
            "bytes" => Ok(ConverterKind::Bytes),
            "utf8" => Ok(ConverterKind::Utf8),
            _ => Err(ConvertError::UnknownConverter {
                name: name.to_string(),
                expected: Self::EXPECTED,
            }),
        }
    }

    /// Runs the forward conversion over the whole `data` byte array — the general
    /// "overall" convert. `from` / `to` supply the dtype arguments the kind needs.
    pub fn convert_bytes(
        self,
        data: &[u8],
        from: Option<PrimitiveType>,
        to: Option<PrimitiveType>,
    ) -> Result<Vec<u8>, ConvertError> {
        match self {
            ConverterKind::Cast => self
                .require(from, "from")?
                .cast_bytes(self.require(to, "to")?, data),
            ConverterKind::String => self.require(from, "dtype")?.string_convert_bytes(data),
            ConverterKind::Bytes => self.require(from, "dtype")?.bytes_convert_bytes(data),
            ConverterKind::Utf8 => Utf8Converter::new().convert_byte_array(data),
        }
    }

    /// Runs the inverse conversion over the whole `data` byte array — the exact inverse
    /// of [`convert_bytes`](ConverterKind::convert_bytes).
    pub fn invert_bytes(
        self,
        data: &[u8],
        from: Option<PrimitiveType>,
        to: Option<PrimitiveType>,
    ) -> Result<Vec<u8>, ConvertError> {
        match self {
            // Cast inverts by casting the target bytes back to the source dtype.
            ConverterKind::Cast => self
                .require(to, "to")?
                .cast_bytes(self.require(from, "from")?, data),
            ConverterKind::String => self.require(from, "dtype")?.string_invert_bytes(data),
            // The bytes codec is a width-checked pass-through, so its inverse is itself.
            ConverterKind::Bytes => self.require(from, "dtype")?.bytes_convert_bytes(data),
            ConverterKind::Utf8 => Utf8Converter::new().invert_byte_array(data),
        }
    }

    /// Unwraps a required dtype argument, or a guided [`ConvertError::MissingDtype`]
    /// naming the argument this kind needs.
    fn require(
        self,
        dtype: Option<PrimitiveType>,
        arg: &'static str,
    ) -> Result<PrimitiveType, ConvertError> {
        dtype.ok_or(ConvertError::MissingDtype {
            kind: self.name(),
            arg,
            expected: PrimitiveType::EXPECTED,
        })
    }
}
