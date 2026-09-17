//! [`Url`] as a scalar value.
//!
//! The value type itself lives in [`crate::uri`] - a URL is what a handle
//! addresses itself by, and there is one of those, not one for locations and
//! another for columns. This is only what makes that value a scalar.

use std::sync::Arc;

use crate::types::scalar::Value;
use crate::{DataType, Result, Scalar, Url};

impl Value for Url {
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

impl From<Url> for Scalar {
    fn from(value: Url) -> Self {
        Self::Url(Arc::new(value))
    }
}
