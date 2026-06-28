//! [`NullSerie`] — a column whose every cell is null, backed by an Arrow
//! [`NullArray`](arrow_array::NullArray). The `Null` Arrow type carries no values, so it
//! is the natural home for an all-null column and the target of casting any column to
//! `null` (a "fast cast" that needs no value conversion).

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, NullArray};
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::scalar::Scalar;
use crate::serie::Serie;

/// An all-null column of [`DataType::Null`]. It stores only a length; every read is
/// [`Scalar::Null`].
#[derive(Debug, Clone)]
pub struct NullSerie {
    field: Field,
    values: NullArray,
}

impl NullSerie {
    /// Wraps a field and array (no validation) — used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, values: NullArray) -> Self {
        NullSerie { field, values }
    }

    /// Builds from a [`Field`] (which must be [`DataType::Null`]) and an array.
    pub fn new(field: Field, values: NullArray) -> SerieResult<Self> {
        if !field.data_type().is_null() {
            return Err(SerieError::TypeMismatch {
                expected: DataType::Null.to_str(),
                found: field.data_type().to_str(),
            });
        }
        Ok(NullSerie::from_parts(field, values))
    }

    /// An all-null column named `name`, `len` rows long.
    pub fn from_len(name: impl Into<String>, len: usize) -> Self {
        let field = Field::new(name, DataType::Null, true);
        NullSerie::from_parts(field, NullArray::new(len))
    }
}

impl Serie for NullSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.len()
    }

    fn is_null(&self, _index: usize) -> bool {
        true
    }

    /// Every cell is null.
    fn value_at(&self, _index: usize) -> Scalar {
        Scalar::Null
    }
}
