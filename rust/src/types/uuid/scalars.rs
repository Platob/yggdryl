//! UUID values and the typed scalar alias.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarFamily, ScalarValue, types};

/// One RFC 9562 identifier stored as its big-endian 128-bit value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Uuid(u128);

const _: () = assert!(std::mem::size_of::<Uuid>() == 16);

impl Uuid {
    /// Construct from the exact packed identifier.
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Parse the accepted hyphenated, compact-hex, or 16-byte representation.
    pub fn from_bytes(value: &[u8]) -> Result<Self> {
        Ok(Self(u128::from_be_bytes(types::uuid_parse(value)?)))
    }

    /// Return the packed identifier.
    pub const fn get(self) -> u128 {
        self.0
    }

    /// Return the canonical sixteen storage bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&types::uuid_text(&self.0.to_be_bytes()))
    }
}

impl ScalarFamily for Uuid {
    const KIND: DataTypeKind = DataTypeKind::Uuid;

    fn id(&self) -> DataTypeId {
        DataTypeId::Uuid
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Uuid)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Uuid(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Uuid(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarValue for Uuid {
    type Family = Self;
    type Type = super::UuidType;

    const ID: DataTypeId = DataTypeId::Uuid;
    const KIND: DataTypeKind = DataTypeKind::Uuid;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Uuid)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Uuid(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}

define_scalar_type!(UuidScalar, super::UuidType, "uuid", crate::DataType::Uuid);
