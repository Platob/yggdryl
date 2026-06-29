//! The [`Utf8`] scalar — a validated, in-memory UTF-8 string value.

use std::collections::BTreeMap;

use crate::buffer::Buffer;
use crate::datatype::{AnyType, DataType, Utf8Type};
use crate::error::ScalarError;
use crate::scalar::{AnyScalar, Binary, Scalar};

/// An in-memory UTF-8 string value.
///
/// The payload lives in a shared [`Buffer`] whose bytes are validated as UTF-8 on
/// every way in (constructors, byte/mapping parsing *and* deserialization), so
/// [`as_str`](Utf8::as_str) hands back a borrowed `&str` without re-validating or
/// copying. Unlike [`Binary`], it is not a mutable byte-IO buffer (that would
/// break the UTF-8 invariant); [`cast`](Scalar::cast) to a `Binary` for IO.
///
/// ```
/// use yggdryl_core::{Scalar, Utf8};
///
/// let s = Utf8::new("yggdryl");
/// assert_eq!(s.as_str(), "yggdryl");
/// assert_eq!(Utf8::from_str("yggdryl"), s);
/// assert_eq!(Utf8::from_bytes(&s.to_bytes()).unwrap(), s);
/// ```
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(from = "Utf8Repr"))]
pub struct Utf8 {
    data_type: Utf8Type,
    buffer: Buffer,
}

impl Utf8 {
    /// A `string` value holding `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            data_type: Utf8Type::new(),
            buffer: Buffer::from(value.into()),
        }
    }

    /// A `string` value holding `value` (the inverse of [`as_str`](Utf8::as_str)).
    #[allow(clippy::should_implement_trait)] // `from_str` is the crate-wide naming convention.
    pub fn from_str(value: &str) -> Self {
        Self {
            data_type: Utf8Type::new(),
            buffer: Buffer::from(value),
        }
    }

    /// A value sharing `buffer` without copying, validating that it holds UTF-8.
    pub fn from_buffer(buffer: Buffer) -> Result<Self, ScalarError> {
        std::str::from_utf8(buffer.as_slice()).map_err(|_| ScalarError::InvalidUtf8)?;
        Ok(Self {
            data_type: Utf8Type::new(),
            buffer,
        })
    }

    /// A `string` value holding a copy of `bytes`, validating UTF-8.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
        Self::from_buffer(Buffer::from_slice(bytes))
    }

    /// The string, borrowed without copying.
    pub fn as_str(&self) -> &str {
        // SAFETY: every constructor and parser validates UTF-8, so the buffer is
        // always valid UTF-8 here.
        unsafe { std::str::from_utf8_unchecked(self.buffer.as_slice()) }
    }

    /// The string's bytes, borrowed without copying.
    pub fn as_bytes(&self) -> &[u8] {
        self.buffer.as_slice()
    }

    /// The number of UTF-8 bytes.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The value's concrete [`Utf8Type`] (`string` vs `large_string`).
    pub fn string_type(&self) -> Utf8Type {
        self.data_type
    }

    /// Returns a copy carrying the given `string` type variant; the payload is
    /// shared, not copied.
    pub fn with_data_type(&self, data_type: Utf8Type) -> Self {
        Self {
            data_type,
            buffer: self.buffer.clone(),
        }
    }

    /// The string's raw UTF-8 bytes as an owned `Vec`. The `string` vs
    /// `large_string` variant round-trips through [`to_mapping`](Utf8::to_mapping)
    /// / [`to_json`](Scalar::to_json), not the raw bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    /// The component map (`type` plus the `value` text).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("type".to_string(), self.data_type.type_name().to_string());
        map.insert("value".to_string(), self.as_str().to_string());
        map
    }

    /// Reconstructs a value from the component map produced by
    /// [`to_mapping`](Utf8::to_mapping).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, ScalarError> {
        let type_name = map
            .get("type")
            .ok_or_else(|| ScalarError::InvalidEncoding("missing \"type\" key".to_string()))?;
        let data_type = Utf8Type::from_str(type_name)?;
        let value = map.get("value").cloned().unwrap_or_default();
        Ok(Self {
            data_type,
            buffer: Buffer::from(value),
        })
    }
}

impl Default for Utf8 {
    fn default() -> Self {
        Self::new("")
    }
}

impl std::fmt::Debug for Utf8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Utf8").field(&self.as_str()).finish()
    }
}

impl Scalar for Utf8 {
    fn data_type(&self) -> AnyType {
        self.data_type.to_any()
    }

    fn set_data_type(&mut self, data_type: &dyn DataType) -> Result<(), ScalarError> {
        match data_type.to_any() {
            AnyType::Utf8(utf8) => {
                self.data_type = utf8;
                Ok(())
            }
            other => Err(ScalarError::IncompatibleType(format!(
                "cannot set type \"{}\" on a string scalar; use cast",
                other.to_str()
            ))),
        }
    }

    fn cast(&self, data_type: &dyn DataType) -> Result<AnyScalar, ScalarError> {
        match data_type.to_any() {
            AnyType::Utf8(utf8) => Ok(AnyScalar::Utf8(self.with_data_type(utf8))),
            AnyType::Binary(binary) => Ok(AnyScalar::Binary(
                Binary::from_bytes(self.as_bytes()).with_data_type(binary),
            )),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Utf8 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Utf8", 2)?;
        state.serialize_field("type", &self.data_type)?;
        state.serialize_field("value", self.as_str())?;
        state.end()
    }
}

/// Deserialization shim: accepts a JSON string `value` and rebuilds the buffer
/// (UTF-8 by construction, so [`Utf8::as_str`] stays sound).
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct Utf8Repr {
    #[serde(rename = "type")]
    data_type: Utf8Type,
    value: String,
}

#[cfg(feature = "serde")]
impl From<Utf8Repr> for Utf8 {
    fn from(repr: Utf8Repr) -> Self {
        Utf8 {
            data_type: repr.data_type,
            buffer: Buffer::from(repr.value),
        }
    }
}
