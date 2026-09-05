//! Null and Boolean values and typed scalar aliases.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, ScalarFamily, ScalarValue};

/// The one null value.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Null;

impl fmt::Display for Null {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null")
    }
}

/// One Boolean value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Boolean(bool);

impl Boolean {
    /// Construct a Boolean value.
    pub const fn new(value: bool) -> Self {
        Self(value)
    }

    /// Return the native Boolean.
    pub const fn get(self) -> bool {
        self.0
    }
}

impl fmt::Display for Boolean {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<bool> for Boolean {
    fn from(value: bool) -> Self {
        Self::new(value)
    }
}

impl From<Boolean> for bool {
    fn from(value: Boolean) -> Self {
        value.get()
    }
}

impl From<()> for Scalar {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Boolean(Boolean::new(value))
    }
}

impl ScalarFamily for Null {
    const KIND: DataTypeKind = DataTypeKind::Null;

    fn id(&self) -> DataTypeId {
        DataTypeId::Null
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Null)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Null
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        static NULL: Null = Null;
        value.is_null().then_some(&NULL)
    }
}

impl ScalarValue for Null {
    type Family = Self;
    type Type = super::NullType;

    const ID: DataTypeId = DataTypeId::Null;
    const KIND: DataTypeKind = DataTypeKind::Null;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Null)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Null
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}

impl ScalarFamily for Boolean {
    const KIND: DataTypeKind = DataTypeKind::Boolean;

    fn id(&self) -> DataTypeId {
        DataTypeId::Boolean
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Boolean(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Boolean(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarValue for Boolean {
    type Family = Self;
    type Type = super::BooleanType;

    const ID: DataTypeId = DataTypeId::Boolean;
    const KIND: DataTypeKind = DataTypeKind::Boolean;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Boolean(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}

define_scalar_type!(NullScalar, super::NullType, "null", crate::DataType::Null);
define_scalar_type!(
    BooleanScalar,
    super::BooleanType,
    "boolean",
    crate::DataType::Boolean
);
