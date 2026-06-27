//! [`VarcharSerie<O>`] — a string column, backed by an Arrow
//! [`GenericStringArray<O>`](arrow_array::GenericStringArray). The offset width `O`
//! selects `Utf8` (`i32`) versus `LargeUtf8` (`i64`).

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, GenericStringArray, OffsetSizeTrait};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::scalar::{scalar_at_ref, Scalar};
use crate::serie::{Serie, TypedSerie};

/// A UTF-8 string column. `O = i32` is `Utf8`, `O = i64` is `LargeUtf8`.
pub struct VarcharSerie<O: OffsetSizeTrait> {
    field: Field,
    values: GenericStringArray<O>,
}

impl<O: OffsetSizeTrait> VarcharSerie<O> {
    /// Wraps a field and array (no validation) — used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, values: GenericStringArray<O>) -> Self {
        VarcharSerie { field, values }
    }

    /// Builds from a [`Field`] and array, checking the field's type maps to the array's
    /// Arrow string type (so the `large` flag matches the offset width `O`).
    pub fn new(field: Field, values: GenericStringArray<O>) -> SerieResult<Self> {
        let expected = field.data_type().to_arrow()?;
        if &expected != values.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: values.data_type().to_string(),
            });
        }
        Ok(VarcharSerie::from_parts(field, values))
    }

    /// Builds a column named `name` from an iterator of optional strings, deriving the
    /// [`Field`] (nullable) from the offset width.
    pub fn from_values<S: AsRef<str>>(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<S>>,
    ) -> Self
    where
        GenericStringArray<O>: FromIterator<Option<S>>,
    {
        let values: GenericStringArray<O> = values.into_iter().collect();
        let afield = arrow_schema::Field::new(name, values.data_type().clone(), true);
        VarcharSerie::from_parts(Field::from_arrow(&afield), values)
    }

    /// The value at `index` as a borrowed `&str`, or `None` when null / out of bounds —
    /// the zero-copy companion to [`get`](TypedSerie::get).
    pub fn str_value(&self, index: usize) -> Option<&str> {
        if index < self.values.len() && self.values.is_valid(index) {
            Some(self.values.value(index))
        } else {
            None
        }
    }

    /// The typed backing [`GenericStringArray<O>`](arrow_array::GenericStringArray).
    pub fn values(&self) -> &GenericStringArray<O> {
        &self.values
    }
}

impl<O: OffsetSizeTrait> Clone for VarcharSerie<O> {
    fn clone(&self) -> Self {
        VarcharSerie {
            field: self.field.clone(),
            values: self.values.clone(),
        }
    }
}

impl<O: OffsetSizeTrait> fmt::Debug for VarcharSerie<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VarcharSerie")
            .field("field", &self.field)
            .field("values", &self.values)
            .finish()
    }
}

impl<O: OffsetSizeTrait> Serie for VarcharSerie<O> {
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

impl<O: OffsetSizeTrait> TypedSerie<String> for VarcharSerie<O> {
    fn get(&self, index: usize) -> Option<String> {
        self.str_value(index).map(str::to_string)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<String>> + '_> {
        Box::new(self.values.iter().map(|opt| opt.map(str::to_string)))
    }
}
