//! The [`BinaryScalar`] — a single binary value (or null).

use crate::buffer::Buffer;
use crate::datatype::{AnyType, Binary, DataType};
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
    /// A non-null scalar holding `value`. Building from an owned `Vec`/`Buffer`
    /// does not copy; building from a borrowed slice copies once.
    pub fn new(value: impl Into<Buffer>) -> Self {
        Self {
            data_type: Binary::new(),
            value: Some(value.into()),
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
    pub fn with_type(&self, data_type: Binary) -> Self {
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

    /// The JSON form (`{"type": …, "value": …}`).
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("BinaryScalar serializes to JSON")
    }

    /// Parses the JSON form produced by [`BinaryScalar::to_json`].
    #[cfg(feature = "json")]
    pub fn from_json(value: &str) -> Result<Self, crate::error::ScalarError> {
        serde_json::from_str(value)
            .map_err(|err| crate::error::ScalarError::InvalidEncoding(err.to_string()))
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
