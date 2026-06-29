//! The [`BinaryScalar`] — a single binary value (or null).

use std::collections::BTreeMap;

use crate::buffer::Buffer;
use crate::datatype::{AnyType, Binary, BinaryBased, DataType};
use crate::error::ScalarError;
use crate::mapping::{decode_hex, encode_hex};
use crate::scalar::Scalar;

/// A single [`Binary`] value, or null.
///
/// The payload lives in a [`Buffer`], so [`as_bytes`](BinaryScalar::as_bytes)
/// borrows it without copying and cloning the scalar is O(1).
///
/// ```
/// use yggdryl_core::{BinaryScalar, Scalar};
///
/// let scalar = BinaryScalar::new(b"\x00\x01\x02".as_slice());
/// assert_eq!(scalar.as_bytes(), Some(b"\x00\x01\x02".as_slice()));
/// assert!(!scalar.is_null());
/// assert_eq!(BinaryScalar::from_bytes(&scalar.to_bytes()).unwrap(), scalar);
/// assert!(BinaryScalar::null().is_null());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BinaryScalar {
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    data_type: Binary,
    value: Option<Buffer>,
}

impl BinaryScalar {
    /// A non-null scalar holding `value`. Building from an existing [`Buffer`] is
    /// O(1) (the payload is shared); building from a `Vec` or a borrowed slice
    /// copies the bytes once into a fresh allocation.
    pub fn new(value: impl Into<Buffer>) -> Self {
        Self {
            data_type: Binary::new(),
            value: Some(value.into()),
        }
    }

    /// A non-null scalar sharing `buffer` without copying.
    pub fn from_buffer(buffer: Buffer) -> Self {
        Self {
            data_type: Binary::new(),
            value: Some(buffer),
        }
    }

    /// The null `binary` scalar.
    pub fn null() -> Self {
        Self {
            data_type: Binary::new(),
            value: None,
        }
    }

    /// Returns a copy of this scalar carrying the given `binary` type variant
    /// (`binary` vs `large_binary`); the payload is shared, not copied.
    pub fn with_data_type(&self, data_type: Binary) -> Self {
        Self {
            data_type,
            value: self.value.clone(),
        }
    }

    /// The scalar's bytes, borrowed without copying; `None` if null.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_ref().map(Buffer::as_slice)
    }

    /// The scalar's backing [`Buffer`]; `None` if null.
    pub fn buffer(&self) -> Option<&Buffer> {
        self.value.as_ref()
    }

    /// The number of bytes, or `None` if null.
    pub fn len(&self) -> Option<usize> {
        self.value.as_ref().map(Buffer::len)
    }

    /// Whether the scalar is non-null and empty.
    pub fn is_empty(&self) -> Option<bool> {
        self.value.as_ref().map(Buffer::is_empty)
    }

    /// The scalar's concrete [`Binary`] type.
    pub fn binary_type(&self) -> Binary {
        self.data_type
    }

    /// The compact binary frame: `[type tag][null flag][raw payload]`. The raw
    /// bytes are stored verbatim (no hex/JSON expansion).
    pub fn to_bytes(&self) -> Vec<u8> {
        let tag = if self.data_type.is_large() { 1u8 } else { 0u8 };
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

    /// Parses the binary frame produced by [`to_bytes`](BinaryScalar::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
        let (&tag, rest) = bytes
            .split_first()
            .ok_or_else(|| ScalarError::InvalidEncoding("empty binary scalar frame".to_string()))?;
        let large = match tag {
            0 => false,
            1 => true,
            other => {
                return Err(ScalarError::InvalidEncoding(format!(
                    "expected a binary type tag (0 or 1), got {other}"
                )))
            }
        };
        let (&null_flag, payload) = rest
            .split_first()
            .ok_or_else(|| ScalarError::InvalidEncoding("missing null flag".to_string()))?;
        let value = match null_flag {
            0 => Some(Buffer::from_slice(payload)),
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
            data_type: if large {
                Binary::large()
            } else {
                Binary::new()
            },
            value,
        })
    }

    /// The component map (`type`, plus `value` as hex or a `null` marker).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("type".to_string(), self.data_type.type_name().to_string());
        match &self.value {
            Some(buffer) => {
                map.insert("value".to_string(), encode_hex(buffer.as_slice()));
            }
            None => {
                map.insert("null".to_string(), "true".to_string());
            }
        }
        map
    }

    /// Reconstructs a scalar from the component map produced by
    /// [`to_mapping`](BinaryScalar::to_mapping).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, ScalarError> {
        let type_name = map
            .get("type")
            .ok_or_else(|| ScalarError::InvalidEncoding("missing \"type\" key".to_string()))?;
        let data_type = Binary::from_str(type_name)?;
        let value = match map.get("value") {
            Some(hex) => Some(Buffer::from_vec(
                decode_hex(hex).map_err(ScalarError::InvalidEncoding)?,
            )),
            None => None,
        };
        Ok(Self { data_type, value })
    }
}

impl Scalar for BinaryScalar {
    fn data_type(&self) -> AnyType {
        self.data_type.to_any()
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}
