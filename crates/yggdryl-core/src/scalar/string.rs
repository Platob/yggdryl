//! The [`StringScalar`] — a single UTF-8 string value (or null).

use std::collections::BTreeMap;

use crate::buffer::Buffer;
use crate::datatype::{AnyType, BinaryBased, DataType, Utf8};
use crate::error::ScalarError;
use crate::scalar::Scalar;

/// A single [`Utf8`] string value, or null.
///
/// The payload lives in a [`Buffer`] whose bytes are validated as UTF-8 on every
/// way in (constructors, byte/mapping parsing *and* deserialization), so
/// [`as_str`](StringScalar::as_str) hands back a borrowed `&str` without
/// re-validating or copying.
///
/// ```
/// use yggdryl_core::{Scalar, StringScalar};
///
/// let scalar = StringScalar::new("yggdryl");
/// assert_eq!(scalar.as_str(), Some("yggdryl"));
/// assert_eq!(StringScalar::from_bytes(&scalar.to_bytes()).unwrap(), scalar);
/// assert!(StringScalar::null().is_null());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(from = "StringScalarRepr"))]
pub struct StringScalar {
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
    pub fn with_data_type(&self, data_type: Utf8) -> Self {
        Self {
            data_type,
            value: self.value.clone(),
        }
    }

    /// The scalar's text, borrowed without copying; `None` if null.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_ref().map(|buffer| {
            // SAFETY: every constructor and parser validates UTF-8, so the buffer
            // is always valid UTF-8 here.
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

    /// The compact binary frame: `[type tag][null flag][raw UTF-8 payload]`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let tag = if self.data_type.is_large() { 3u8 } else { 2u8 };
        match &self.value {
            Some(buffer) => {
                let mut out = Vec::with_capacity(2 + buffer.len());
                out.push(tag);
                out.push(0); // not null
                out.extend_from_slice(buffer.as_slice());
                out
            }
            None => vec![tag, 1],
        }
    }

    /// Parses the binary frame produced by [`to_bytes`](StringScalar::to_bytes),
    /// re-validating UTF-8.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
        let (&tag, rest) = bytes
            .split_first()
            .ok_or_else(|| ScalarError::InvalidEncoding("empty string scalar frame".to_string()))?;
        let large = match tag {
            2 => false,
            3 => true,
            other => {
                return Err(ScalarError::InvalidEncoding(format!(
                    "expected a string type tag (2 or 3), got {other}"
                )))
            }
        };
        let (&null_flag, payload) = rest
            .split_first()
            .ok_or_else(|| ScalarError::InvalidEncoding("missing null flag".to_string()))?;
        let value = match null_flag {
            0 => {
                std::str::from_utf8(payload).map_err(|_| ScalarError::InvalidUtf8)?;
                Some(Buffer::from_slice(payload))
            }
            1 if payload.is_empty() => None,
            1 => {
                return Err(ScalarError::InvalidEncoding(
                    "a null scalar must carry no payload".to_string(),
                ))
            }
            other => {
                return Err(ScalarError::InvalidEncoding(format!(
                    "null flag must be 0 or 1, got {other}"
                )))
            }
        };
        Ok(Self {
            data_type: if large { Utf8::large() } else { Utf8::new() },
            value,
        })
    }

    /// The component map (`type`, plus `value` as text or a `null` marker).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("type".to_string(), self.data_type.type_name().to_string());
        match self.as_str() {
            Some(text) => {
                map.insert("value".to_string(), text.to_string());
            }
            None => {
                map.insert("null".to_string(), "true".to_string());
            }
        }
        map
    }

    /// Reconstructs a scalar from the component map produced by
    /// [`to_mapping`](StringScalar::to_mapping).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, ScalarError> {
        let type_name = map
            .get("type")
            .ok_or_else(|| ScalarError::InvalidEncoding("missing \"type\" key".to_string()))?;
        let data_type = Utf8::from_str(type_name)?;
        let value = map.get("value").map(|text| Buffer::from(text.clone()));
        Ok(Self { data_type, value })
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

/// Serializes a string scalar with a human-readable `value` (a JSON string),
/// not the raw byte array a `Buffer` would otherwise produce.
#[cfg(feature = "serde")]
impl serde::Serialize for StringScalar {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("StringScalar", 2)?;
        state.serialize_field("type", &self.data_type)?;
        state.serialize_field("value", &self.as_str())?;
        state.end()
    }
}

/// Deserialization shim: accepts a JSON string `value` and rebuilds the buffer
/// (UTF-8 by construction, so [`StringScalar::as_str`] stays sound).
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct StringScalarRepr {
    #[serde(rename = "type")]
    data_type: Utf8,
    value: Option<String>,
}

#[cfg(feature = "serde")]
impl From<StringScalarRepr> for StringScalar {
    fn from(repr: StringScalarRepr) -> Self {
        StringScalar {
            data_type: repr.data_type,
            value: repr.value.map(Buffer::from),
        }
    }
}
