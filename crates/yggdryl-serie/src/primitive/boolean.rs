//! [`BooleanSerie`] — a column of `true` / `false`, backed by an Arrow
//! [`BooleanArray`](arrow_array::BooleanArray).

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray};
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::scalar::{scalar_at_ref, Scalar};
use crate::serie::{Serie, TypedSerie};

/// A boolean column.
#[derive(Debug, Clone)]
pub struct BooleanSerie {
    field: Field,
    values: BooleanArray,
}

impl BooleanSerie {
    /// Wraps a field and array (no validation) — used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, values: BooleanArray) -> Self {
        BooleanSerie { field, values }
    }

    /// Builds from a [`Field`] (which must be [`DataType::Boolean`]) and an array.
    pub fn new(field: Field, values: BooleanArray) -> SerieResult<Self> {
        if !field.data_type().is_boolean() {
            return Err(SerieError::TypeMismatch {
                expected: DataType::Boolean.to_str(),
                found: field.data_type().to_str(),
            });
        }
        Ok(BooleanSerie::from_parts(field, values))
    }

    /// Builds a column named `name` from an iterator of optional booleans.
    pub fn from_values(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<bool>>,
    ) -> Self {
        let values: BooleanArray = values.into_iter().collect();
        let field = Field::new(name, DataType::Boolean, true);
        BooleanSerie::from_parts(field, values)
    }

    /// The typed backing [`BooleanArray`](arrow_array::BooleanArray).
    pub fn values(&self) -> &BooleanArray {
        &self.values
    }
}

impl Serie for BooleanSerie {
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
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.values.len() || self.values.is_null(index)
    }

    /// Reads the cell straight off the typed array (no `Arc` clone of [`array`](Serie::array)).
    fn value_at(&self, index: usize) -> Scalar {
        scalar_at_ref(&self.values, index)
    }
}

impl TypedSerie<bool> for BooleanSerie {
    fn get(&self, index: usize) -> Option<bool> {
        if index < self.values.len() && self.values.is_valid(index) {
            Some(self.values.value(index))
        } else {
            None
        }
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<bool>> + '_> {
        Box::new(self.values.iter())
    }
}
