//! [`Url`] as a scalar value.
//!
//! The value type itself lives in [`crate::uri`] - a URL is what a handle
//! addresses itself by, and there is one of those, not one for locations and
//! another for columns. This is only what makes that value a scalar.

use std::sync::Arc;

use crate::types::scalar::{ScalarFamily, ScalarValue};
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, Url};

impl ScalarFamily for Url {
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn id(&self) -> DataTypeId {
        DataTypeId::Url
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Url)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Url(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Url(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarValue for Url {
    type Family = Self;

    const ID: DataTypeId = DataTypeId::Url;
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Url)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Url(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}

impl From<Url> for Scalar {
    fn from(value: Url) -> Self {
        Self::Url(Arc::new(value))
    }
}
