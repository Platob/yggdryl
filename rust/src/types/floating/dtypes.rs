//! Floating-point datatype family.

use smol_str::format_smolstr;

use crate::{DataType, DataTypeId, Error};

/// One Arrow floating-point datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum FloatingType {
    /// IEEE binary16.
    Float16,
    /// IEEE binary32.
    Float32,
    /// IEEE binary64.
    Float64,
}

impl FloatingType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Float16 => DataTypeId::Float16,
            Self::Float32 => DataTypeId::Float32,
            Self::Float64 => DataTypeId::Float64,
        }
    }
}

impl From<FloatingType> for DataType {
    fn from(value: FloatingType) -> Self {
        match value {
            FloatingType::Float16 => Self::Float16,
            FloatingType::Float32 => Self::Float32,
            FloatingType::Float64 => Self::Float64,
        }
    }
}

impl TryFrom<&DataType> for FloatingType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self, Self::Error> {
        match value {
            DataType::Float16 => Ok(Self::Float16),
            DataType::Float32 => Ok(Self::Float32),
            DataType::Float64 => Ok(Self::Float64),
            other => Err(Error::InvalidDataType {
                kind: "floating",
                reason: format_smolstr!("expected a floating datatype, got {other}"),
            }),
        }
    }
}
