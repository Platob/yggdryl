//! The [`PrimitiveType`] family: the flat, child-less Arrow types — the integer,
//! floating-point, boolean, binary and string scalars.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::parse::{split_head, Head};

use super::DataTypeError;

/// A flat, child-less Arrow type: a fixed-width number/boolean, the null type, or
/// a variable-width byte/string buffer. These are the leaves of a schema — they
/// carry no nested [`Field`](crate::Field)s (the one parameter,
/// [`FixedSizeBinary`](PrimitiveType::FixedSizeBinary)'s byte width, is a scalar).
///
/// ```
/// use yggdryl_saga::PrimitiveType;
///
/// assert_eq!(PrimitiveType::from_str("int64").unwrap(), PrimitiveType::Int64);
/// assert_eq!(PrimitiveType::Float64.to_str(), "float64");
/// assert!(PrimitiveType::UInt32.is_numeric());
/// assert_eq!(
///     PrimitiveType::from_str("fixed_size_binary(16)").unwrap(),
///     PrimitiveType::FixedSizeBinary(16)
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PrimitiveType {
    /// The null type — a column of only nulls (`null`).
    Null,
    /// A boolean (`boolean`).
    Boolean,
    /// A signed 8-bit integer (`int8`).
    Int8,
    /// A signed 16-bit integer (`int16`).
    Int16,
    /// A signed 32-bit integer (`int32`).
    Int32,
    /// A signed 64-bit integer (`int64`).
    Int64,
    /// An unsigned 8-bit integer (`uint8`).
    UInt8,
    /// An unsigned 16-bit integer (`uint16`).
    UInt16,
    /// An unsigned 32-bit integer (`uint32`).
    UInt32,
    /// An unsigned 64-bit integer (`uint64`).
    UInt64,
    /// A half-precision (16-bit) float (`float16`).
    Float16,
    /// A single-precision (32-bit) float (`float32`).
    Float32,
    /// A double-precision (64-bit) float (`float64`).
    Float64,
    /// A variable-length byte buffer with 32-bit offsets (`binary`).
    Binary,
    /// A variable-length byte buffer with 64-bit offsets (`large_binary`).
    LargeBinary,
    /// A variable-length byte buffer in the view layout (`binary_view`).
    BinaryView,
    /// A fixed-width byte buffer of the given byte length (`fixed_size_binary(n)`).
    FixedSizeBinary(i32),
    /// A variable-length UTF-8 string with 32-bit offsets (`utf8`).
    Utf8,
    /// A variable-length UTF-8 string with 64-bit offsets (`large_utf8`).
    LargeUtf8,
    /// A variable-length UTF-8 string in the view layout (`utf8_view`).
    Utf8View,
}

impl PrimitiveType {
    /// `true` for the integer and floating-point types (everything numeric).
    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_floating()
    }

    /// `true` for the signed and unsigned integer types.
    pub fn is_integer(&self) -> bool {
        use PrimitiveType::*;
        matches!(
            self,
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64
        )
    }

    /// `true` for the floating-point types.
    pub fn is_floating(&self) -> bool {
        matches!(
            self,
            PrimitiveType::Float16 | PrimitiveType::Float32 | PrimitiveType::Float64
        )
    }

    /// `true` for the variable-length string types (`utf8` / `large_utf8` /
    /// `utf8_view`).
    pub fn is_string(&self) -> bool {
        matches!(
            self,
            PrimitiveType::Utf8 | PrimitiveType::LargeUtf8 | PrimitiveType::Utf8View
        )
    }

    /// Parses a canonical primitive name (e.g. `int64`, `utf8`,
    /// `fixed_size_binary(16)`), accepting the common aliases (`bool`, `string`,
    /// `double`). Returns [`DataTypeError::Unknown`] for a name that is not a
    /// primitive, so [`DataType::from_str`](crate::DataType::from_str) can try the
    /// other families.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<PrimitiveType, DataTypeError> {
        log_event!(trace, "PrimitiveType::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DataTypeError::Empty);
        }
        let head =
            split_head(trimmed).ok_or_else(|| DataTypeError::Invalid(trimmed.to_string()))?;
        PrimitiveType::from_head(&head)
    }

    /// Builds a primitive from a parsed [`Head`]. Names not owned by this family
    /// return [`DataTypeError::Unknown`]; an owned name with the wrong
    /// params/body returns [`DataTypeError::Invalid`].
    pub(crate) fn from_head(head: &Head) -> Result<PrimitiveType, DataTypeError> {
        use PrimitiveType::*;
        // The one parametric primitive.
        if head.name == "fixed_size_binary" {
            if head.body.is_some() {
                return Err(DataTypeError::Invalid(
                    "'fixed_size_binary' takes no <body>".to_string(),
                ));
            }
            let width = head
                .params
                .ok_or_else(|| {
                    DataTypeError::Invalid(
                        "'fixed_size_binary' needs a byte width, e.g. fixed_size_binary(16)"
                            .to_string(),
                    )
                })?
                .trim()
                .parse::<i32>()
                .map_err(|_| {
                    DataTypeError::Invalid("fixed_size_binary width must be an integer".to_string())
                })?;
            return Ok(FixedSizeBinary(width));
        }

        let ty = match head.name {
            "null" => Null,
            "bool" | "boolean" => Boolean,
            "int8" => Int8,
            "int16" => Int16,
            "int32" => Int32,
            "int64" => Int64,
            "uint8" => UInt8,
            "uint16" => UInt16,
            "uint32" => UInt32,
            "uint64" => UInt64,
            "float16" | "halffloat" => Float16,
            "float32" | "float" => Float32,
            "float64" | "double" => Float64,
            "binary" => Binary,
            "large_binary" => LargeBinary,
            "binary_view" => BinaryView,
            "utf8" | "string" => Utf8,
            "large_utf8" | "large_string" => LargeUtf8,
            "utf8_view" => Utf8View,
            _ => return Err(DataTypeError::Unknown(head.name.to_string())),
        };
        if head.params.is_some() || head.body.is_some() {
            return Err(DataTypeError::Invalid(format!(
                "'{}' takes no parameters",
                head.name
            )));
        }
        Ok(ty)
    }

    /// Renders the canonical name — the inverse of [`from_str`](PrimitiveType::from_str).
    pub fn to_str(&self) -> String {
        use PrimitiveType::*;
        match self {
            Null => "null".to_string(),
            Boolean => "boolean".to_string(),
            Int8 => "int8".to_string(),
            Int16 => "int16".to_string(),
            Int32 => "int32".to_string(),
            Int64 => "int64".to_string(),
            UInt8 => "uint8".to_string(),
            UInt16 => "uint16".to_string(),
            UInt32 => "uint32".to_string(),
            UInt64 => "uint64".to_string(),
            Float16 => "float16".to_string(),
            Float32 => "float32".to_string(),
            Float64 => "float64".to_string(),
            Binary => "binary".to_string(),
            LargeBinary => "large_binary".to_string(),
            BinaryView => "binary_view".to_string(),
            FixedSizeBinary(n) => format!("fixed_size_binary({n})"),
            Utf8 => "utf8".to_string(),
            LargeUtf8 => "large_utf8".to_string(),
            Utf8View => "utf8_view".to_string(),
        }
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

/// Conversion to the matching `arrow_schema::DataType` (infallible — every
/// primitive has an exact Arrow counterpart).
#[cfg(feature = "arrow")]
impl From<&PrimitiveType> for arrow_schema::DataType {
    fn from(p: &PrimitiveType) -> arrow_schema::DataType {
        use arrow_schema::DataType as A;
        use PrimitiveType::*;
        match p {
            Null => A::Null,
            Boolean => A::Boolean,
            Int8 => A::Int8,
            Int16 => A::Int16,
            Int32 => A::Int32,
            Int64 => A::Int64,
            UInt8 => A::UInt8,
            UInt16 => A::UInt16,
            UInt32 => A::UInt32,
            UInt64 => A::UInt64,
            Float16 => A::Float16,
            Float32 => A::Float32,
            Float64 => A::Float64,
            Binary => A::Binary,
            LargeBinary => A::LargeBinary,
            BinaryView => A::BinaryView,
            FixedSizeBinary(n) => A::FixedSizeBinary(*n),
            Utf8 => A::Utf8,
            LargeUtf8 => A::LargeUtf8,
            Utf8View => A::Utf8View,
        }
    }
}
