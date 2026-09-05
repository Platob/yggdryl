//! Validated binary datatype construction.

use crate::types::validate_non_negative;
use crate::{DataType, DataTypeId, Error, Result};
use smol_str::format_smolstr;

/// One Arrow binary datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BytesType {
    /// Binary with 32-bit offsets.
    Binary,
    /// Fixed-width binary.
    FixedSizeBinary(i32),
    /// Binary with 64-bit offsets.
    LargeBinary,
    /// Binary view layout.
    BinaryView,
}

impl BytesType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Binary => DataTypeId::Binary,
            Self::FixedSizeBinary(_) => DataTypeId::FixedSizeBinary,
            Self::LargeBinary => DataTypeId::LargeBinary,
            Self::BinaryView => DataTypeId::BinaryView,
        }
    }

    /// Validate and convert this family member into the root datatype.
    pub fn into_dtype(self) -> Result<DataType> {
        match self {
            Self::FixedSizeBinary(width) => DataType::fixed_size_binary(width),
            other => Ok(other.into()),
        }
    }
}

impl From<BytesType> for DataType {
    fn from(value: BytesType) -> Self {
        match value {
            BytesType::Binary => Self::Binary,
            BytesType::FixedSizeBinary(width) => Self::FixedSizeBinary(width),
            BytesType::LargeBinary => Self::LargeBinary,
            BytesType::BinaryView => Self::BinaryView,
        }
    }
}

impl TryFrom<&DataType> for BytesType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Binary => Ok(Self::Binary),
            DataType::FixedSizeBinary(width) => Ok(Self::FixedSizeBinary(*width)),
            DataType::LargeBinary => Ok(Self::LargeBinary),
            DataType::BinaryView => Ok(Self::BinaryView),
            other => Err(Error::InvalidDataType {
                kind: "bytes",
                reason: format_smolstr!("expected a bytes datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Creates a fixed-size binary type after validating its width.
    pub fn fixed_size_binary(width: i32) -> Result<Self> {
        validate_non_negative("FixedSizeBinary", "width", width)?;
        Ok(Self::FixedSizeBinary(width))
    }
}
