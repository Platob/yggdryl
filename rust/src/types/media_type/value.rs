//! [`MediaType`] as a scalar value.
//!
//! The value type itself lives beside [`crate::MediaType`]; this is only what
//! makes it a scalar. It is held behind one shared pointer: a media type
//! carries a base, a charset and a coding list, which is wider than the scalar
//! enum, and a column of them is cloned once per row.

use std::sync::Arc;

use crate::types::scalar::Value;
use crate::{DataType, MediaType, Result, Scalar};

impl Value for MediaType {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::MediaType)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::MediaType(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::MediaType(value) => Some(value),
            _ => None,
        }
    }
}

impl From<MediaType> for Scalar {
    fn from(value: MediaType) -> Self {
        Self::MediaType(Arc::new(value))
    }
}
