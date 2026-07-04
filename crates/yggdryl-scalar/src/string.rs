//! The [`StringScalar`] scalar.

use crate::{Scalar, ScalarFactory, TypedScalar};
use yggdryl_core::StringBuffer;
use yggdryl_dtype::{DataError, StringType};

/// A single, possibly-null `utf8` value: a string held as a core [`StringBuffer`],
/// so the value doubles as a positioned-IO resource with a typed `char` view.
///
/// It is the string counterpart of [`BinaryScalar`](crate::BinaryScalar): where a
/// binary value is a [`ByteBuffer`](yggdryl_core::ByteBuffer), a string value is a
/// [`StringBuffer`](yggdryl_core::StringBuffer) — the same UTF-8 bytes, plus the
/// `char`-typed [`IOBase<char>`](yggdryl_core::IOBase). [`value`](Scalar::value) /
/// [`as_str`](Scalar::as_str) borrow the string directly (never copying),
/// [`as_bytes`](Scalar::as_bytes) its UTF-8 bytes, and [`io`](StringScalar::io) /
/// [`into_io`](StringScalar::into_io) hand back the [`StringBuffer`] for positioned
/// reads and char writes. Crossing the Arrow boundary copies the bytes once between
/// the Arrow `utf8` buffer and the core resource.
///
/// ```
/// use yggdryl_core::{IOBase, RawIOBase, Whence};
/// use yggdryl_scalar::yggdryl_dtype::DataType;
/// use yggdryl_scalar::{Scalar, StringScalar};
///
/// let greeting = StringScalar::new("hé".to_string());
/// assert!(!greeting.is_null());
/// assert_eq!(greeting.value(), Some("hé"));
/// assert_eq!(greeting.as_bytes().unwrap(), &[b'h', 0xC3, 0xA9][..]); // UTF-8 bytes
/// assert_eq!(greeting.data_type().name(), "utf8");
///
/// // The value is a core IO resource: positioned byte reads and a typed char view.
/// let io = greeting.io().unwrap();
/// assert_eq!(io.byte_size(), 3);
/// assert_eq!(IOBase::<char>::size(io), 2); // two chars, three bytes
///
/// // The Arrow round trip is exact (Arrow's Utf8).
/// let arrow = greeting.to_arrow_scalar();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(StringScalar::from_arrow(arrow.as_ref()).unwrap(), greeting);
///
/// assert!(StringScalar::null().is_null());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringScalar {
    data_type: StringType,
    value: Option<StringBuffer>,
}

impl StringScalar {
    /// A `utf8` scalar holding `value` (an empty string is the empty value, not
    /// null).
    pub fn new(value: String) -> Self {
        Self {
            data_type: StringType,
            value: Some(StringBuffer::from_string(value)),
        }
    }

    /// A null `utf8` scalar.
    pub fn null() -> Self {
        Self {
            data_type: StringType,
            value: None,
        }
    }

    /// The value as the core positioned-IO resource, borrowed — every
    /// [`RawIOBase`](yggdryl_core::RawIOBase) read and typed
    /// [`IOBase<char>`](yggdryl_core::IOBase) access works on the borrow — or `None`
    /// when null.
    pub fn io(&self) -> Option<&StringBuffer> {
        self.value.as_ref()
    }

    /// Consume the scalar, returning the value as the core [`StringBuffer`] (or
    /// `None` when null) — ready to wrap in a cursor / slice adapter.
    pub fn into_io(self) -> Option<StringBuffer> {
        self.value
    }
}

impl From<String> for StringScalar {
    /// A `utf8` scalar holding `value`.
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for StringScalar {
    /// A `utf8` scalar holding a copy of `value`.
    fn from(value: &str) -> Self {
        Self::new(value.to_string())
    }
}

impl From<Option<String>> for StringScalar {
    /// A `utf8` scalar holding `value`, or the null scalar for `None`.
    fn from(value: Option<String>) -> Self {
        match value {
            Some(value) => Self::new(value),
            None => Self::null(),
        }
    }
}

impl From<StringBuffer> for StringScalar {
    /// A `utf8` scalar taking over an existing core IO resource, moved — the inverse
    /// of [`into_io`](StringScalar::into_io).
    fn from(value: StringBuffer) -> Self {
        Self {
            data_type: StringType,
            value: Some(value),
        }
    }
}

impl Scalar for StringScalar {
    type DataType = StringType;
    type Value = str;

    fn data_type(&self) -> &StringType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Option<&str> {
        // The buffer is always valid UTF-8 (built from a `String`), so `as_str`
        // succeeds; a raw byte write is the only way to invalidate it.
        self.value.as_ref().and_then(|value| value.as_str().ok())
    }

    fn to_arrow_scalar(&self) -> arrow_array::ArrayRef {
        match self.value.as_ref().and_then(|value| value.as_str().ok()) {
            Some(text) => std::sync::Arc::new(arrow_array::StringArray::from_iter_values([text])),
            // Arrow arrays are immutable, so every null scalar shares one cached
            // one-null array; a clone is a reference-count bump.
            None => {
                static NULL: std::sync::OnceLock<arrow_array::ArrayRef> =
                    std::sync::OnceLock::new();
                NULL.get_or_init(|| std::sync::Arc::new(arrow_array::StringArray::new_null(1)))
                    .clone()
            }
        }
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .ok_or_else(|| DataError::IncompatibleArrowType {
                expected: "StringType".to_string(),
                got: arrow_array::Array::data_type(array).to_string(),
            })?;
        Ok(if arrow_array::Array::is_null(array, 0) {
            Self::null()
        } else {
            Self::new(array.value(0).to_string())
        })
    }

    // The native type answers directly, borrowed; the UTF-8 bytes are the value's.
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        self.value
            .as_ref()
            .map(StringBuffer::as_bytes)
            .ok_or(DataError::NullValue)
    }

    fn as_str(
        &self,
        charset: Option<&dyn yggdryl_core::Charset>,
    ) -> Result<std::borrow::Cow<'_, str>, DataError> {
        let value = self.value.as_ref().ok_or(DataError::NullValue)?;
        match charset {
            // The default: the string itself, borrowed straight from the buffer.
            None => value
                .as_str()
                .map(std::borrow::Cow::Borrowed)
                .map_err(DataError::from),
            // An explicit charset re-decodes the UTF-8 bytes through it, owned.
            Some(charset) => charset
                .decode_bytes(value.as_bytes())
                .map(std::borrow::Cow::Owned)
                .map_err(|error| DataError::InexactConversion {
                    value: format!(
                        "{} byte(s) the charset cannot decode ({error})",
                        value.as_bytes().len()
                    ),
                    target: "str",
                }),
        }
    }
}

impl TypedScalar<StringType, str, arrow_array::StringArray> for StringScalar {}

impl ScalarFactory<String> for StringType {
    type Scalar = StringScalar;

    /// A `utf8` scalar holding `value`.
    fn scalar(&self, value: String) -> StringScalar {
        StringScalar::new(value)
    }

    /// The null `utf8` scalar.
    fn null_scalar(&self) -> StringScalar {
        StringScalar::null()
    }

    /// The default `utf8` scalar: the empty string.
    fn default_scalar(&self) -> StringScalar {
        StringScalar::new(String::new())
    }
}
