//! The fixed-size binary data type.

use core::fmt;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId};

/// Opaque bytes of a fixed size per value, mapping to Arrow
/// `FixedSizeBinaryType(size)`.
///
/// ```
/// use yggdryl_schema::{DataType, FixedSizeBinaryType};
///
/// let uuid = FixedSizeBinaryType::from_parts(16).unwrap();
/// assert_eq!(uuid.to_arrow(), arrow_schema::DataType::FixedSizeBinary(16));
/// assert_eq!(FixedSizeBinaryType::from_arrow(&uuid.to_arrow()), Ok(uuid));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawFixedSizeBinary")
)]
pub struct FixedSizeBinaryType {
    size: i32,
}

impl FixedSizeBinaryType {
    /// Builds the type from its per-value byte size, rejecting negative sizes.
    ///
    /// ```
    /// use yggdryl_schema::FixedSizeBinaryType;
    ///
    /// assert!(FixedSizeBinaryType::from_parts(-1).is_err()); // expected 0 or more
    /// ```
    pub fn from_parts(size: i32) -> Result<Self, DataTypeError> {
        if size < 0 {
            return Err(DataTypeError::NegativeFixedSize { size });
        }
        Ok(Self { size })
    }

    /// The byte size of one value.
    pub fn size(&self) -> i32 {
        self.size
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, size: Option<i32>) -> Result<Self, DataTypeError> {
        Self::from_parts(size.unwrap_or(self.size))
    }

    /// Returns a copy with the size replaced.
    pub fn with_size(&self, size: i32) -> Result<Self, DataTypeError> {
        self.copy(Some(size))
    }
}

impl DataType for FixedSizeBinaryType {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::FixedSizeBinary
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::FixedSizeBinary(self.size)
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::FixedSizeBinary(size) => Self::from_parts(*size),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "fixed_size_binary",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![DataTypeId::FixedSizeBinary.to_u8()];
        out.extend_from_slice(&self.size.to_le_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let payload = DataTypeId::FixedSizeBinary.strip_tag(bytes)?;
        let size: [u8; 4] = payload
            .try_into()
            .map_err(|_| DataTypeError::InvalidByteLength {
                expected: 4,
                actual: payload.len(),
            })?;
        Self::from_parts(i32::from_le_bytes(size))
    }
}

impl fmt::Display for FixedSizeBinaryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fixed_size_binary({})", self.size)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawFixedSizeBinary {
    size: i32,
}

#[cfg(feature = "serde")]
impl TryFrom<RawFixedSizeBinary> for FixedSizeBinaryType {
    type Error = DataTypeError;

    fn try_from(raw: RawFixedSizeBinary) -> Result<Self, Self::Error> {
        Self::from_parts(raw.size)
    }
}
