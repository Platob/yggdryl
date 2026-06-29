//! The [`StringScalar`] — a single UTF-8 string value (or null).

use crate::buffer::Buffer;
use crate::datatype::{AnyType, DataType, Utf8};
use crate::error::ScalarError;
use crate::scalar::Scalar;

/// A single [`Utf8`] string value, or null.
///
/// The payload lives in a [`Buffer`] whose bytes are validated as UTF-8 on every
/// way in (constructors *and* deserialization), so [`as_str`](StringScalar::as_str)
/// hands back a borrowed `&str` without re-validating or copying.
///
/// ```
/// use yggdryl_core::{Scalar, StringScalar};
///
/// let scalar = StringScalar::new("yggdryl");
/// assert_eq!(scalar.as_str(), Some("yggdryl"));
/// assert!(StringScalar::null().is_null());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "StringScalarRepr"))]
pub struct StringScalar {
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    data_type: Utf8,
    value: Option<Buffer>,
}

impl StringScalar {
    /// A non-null scalar holding `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            data_type: Utf8::new(),
            value: Some(Buffer::from(value.into())),
        }
    }

    /// The null `string` scalar.
    pub fn null() -> Self {
        Self {
            data_type: Utf8::new(),
            value: None,
        }
    }

    /// Builds a scalar directly over `buffer` without copying, validating that it
    /// holds UTF-8.
    pub fn from_buffer(buffer: Buffer) -> Result<Self, ScalarError> {
        std::str::from_utf8(buffer.as_slice()).map_err(|_| ScalarError::InvalidUtf8)?;
        Ok(Self {
            data_type: Utf8::new(),
            value: Some(buffer),
        })
    }

    /// Returns a copy of this scalar carrying the given `string` type variant
    /// (`string` vs `large_string`); the payload is shared, not copied.
    pub fn with_type(&self, data_type: Utf8) -> Self {
        Self {
            data_type,
            value: self.value.clone(),
        }
    }

    /// The scalar's text, borrowed without copying; `None` if null.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_ref().map(|buffer| {
            // SAFETY: every constructor and the deserializer validate UTF-8, so
            // the buffer is always valid UTF-8 here.
            unsafe { std::str::from_utf8_unchecked(buffer.as_slice()) }
        })
    }

    /// The scalar's bytes, borrowed without copying; `None` if null.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_ref().map(Buffer::as_slice)
    }

    /// The scalar's backing [`Buffer`]; `None` if null.
    pub fn buffer(&self) -> Option<&Buffer> {
        self.value.as_ref()
    }

    /// The scalar's concrete [`Utf8`] type.
    pub fn string_type(&self) -> Utf8 {
        self.data_type
    }

    /// The JSON form (`{"type": …, "value": …}`).
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("StringScalar serializes to JSON")
    }

    /// Parses the JSON form produced by [`StringScalar::to_json`].
    #[cfg(feature = "json")]
    pub fn from_json(value: &str) -> Result<Self, ScalarError> {
        serde_json::from_str(value).map_err(|err| ScalarError::InvalidEncoding(err.to_string()))
    }
}

impl Scalar for StringScalar {
    fn data_type(&self) -> AnyType {
        self.data_type.to_any()
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

/// Deserialization shim that re-validates the UTF-8 invariant before a
/// [`StringScalar`] is reconstructed (so [`StringScalar::as_str`] stays sound).
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct StringScalarRepr {
    #[serde(rename = "type")]
    data_type: Utf8,
    value: Option<Buffer>,
}

#[cfg(feature = "serde")]
impl TryFrom<StringScalarRepr> for StringScalar {
    type Error = String;

    fn try_from(repr: StringScalarRepr) -> Result<Self, Self::Error> {
        if let Some(buffer) = &repr.value {
            std::str::from_utf8(buffer.as_slice())
                .map_err(|_| "string scalar bytes were not valid UTF-8".to_string())?;
        }
        Ok(StringScalar {
            data_type: repr.data_type,
            value: repr.value,
        })
    }
}
