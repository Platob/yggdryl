//! Integer datatype family and predicates used by run-end validation.

use smol_str::format_smolstr;

use crate::{DataType, DataTypeId, Error};

/// One Arrow integer datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IntegerType {
    /// Signed 8-bit integer.
    Int8,
    /// Signed 16-bit integer.
    Int16,
    /// Signed 32-bit integer.
    Int32,
    /// Signed 64-bit integer.
    Int64,
    /// Unsigned 8-bit integer.
    UInt8,
    /// Unsigned 16-bit integer.
    UInt16,
    /// Unsigned 32-bit integer.
    UInt32,
    /// Unsigned 64-bit integer.
    UInt64,
}

impl IntegerType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Int8 => DataTypeId::Int8,
            Self::Int16 => DataTypeId::Int16,
            Self::Int32 => DataTypeId::Int32,
            Self::Int64 => DataTypeId::Int64,
            Self::UInt8 => DataTypeId::UInt8,
            Self::UInt16 => DataTypeId::UInt16,
            Self::UInt32 => DataTypeId::UInt32,
            Self::UInt64 => DataTypeId::UInt64,
        }
    }
}

impl From<IntegerType> for DataType {
    fn from(value: IntegerType) -> Self {
        match value {
            IntegerType::Int8 => Self::Int8,
            IntegerType::Int16 => Self::Int16,
            IntegerType::Int32 => Self::Int32,
            IntegerType::Int64 => Self::Int64,
            IntegerType::UInt8 => Self::UInt8,
            IntegerType::UInt16 => Self::UInt16,
            IntegerType::UInt32 => Self::UInt32,
            IntegerType::UInt64 => Self::UInt64,
        }
    }
}

impl TryFrom<&DataType> for IntegerType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self, Self::Error> {
        match value {
            DataType::Int8 => Ok(Self::Int8),
            DataType::Int16 => Ok(Self::Int16),
            DataType::Int32 => Ok(Self::Int32),
            DataType::Int64 => Ok(Self::Int64),
            DataType::UInt8 => Ok(Self::UInt8),
            DataType::UInt16 => Ok(Self::UInt16),
            DataType::UInt32 => Ok(Self::UInt32),
            DataType::UInt64 => Ok(Self::UInt64),
            other => Err(Error::InvalidDataType {
                kind: "integer",
                reason: format_smolstr!("expected an integer datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Returns whether this is a signed or unsigned integer type.
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::Int8
                | Self::Int16
                | Self::Int32
                | Self::Int64
                | Self::UInt8
                | Self::UInt16
                | Self::UInt32
                | Self::UInt64
        )
    }

    pub(crate) const fn is_run_ends_type(&self) -> bool {
        matches!(self, Self::Int16 | Self::Int32 | Self::Int64)
    }
}
