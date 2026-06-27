//! [`BinarySerie<O>`] — an opaque-bytes column, backed by an Arrow
//! [`GenericBinaryArray<O>`](arrow_array::GenericBinaryArray). The offset width `O`
//! selects `Binary` (`i32`) versus `LargeBinary` (`i64`).

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, GenericBinaryArray, OffsetSizeTrait};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::serie::{Serie, TypedSerie};

/// A binary column. `O = i32` is `Binary`, `O = i64` is `LargeBinary`.
pub struct BinarySerie<O: OffsetSizeTrait> {
    field: Field,
    values: GenericBinaryArray<O>,
}

impl<O: OffsetSizeTrait> BinarySerie<O> {
    /// Wraps a field and array (no validation) — used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, values: GenericBinaryArray<O>) -> Self {
        BinarySerie { field, values }
    }

    /// Builds from a [`Field`] and array, checking the field's type maps to the array's
    /// Arrow binary type (so the `large` flag matches the offset width `O`).
    pub fn new(field: Field, values: GenericBinaryArray<O>) -> SerieResult<Self> {
        let expected = field.data_type().to_arrow()?;
        if &expected != values.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: values.data_type().to_string(),
            });
        }
        Ok(BinarySerie::from_parts(field, values))
    }

    /// Builds a column named `name` from an iterator of optional byte slices, deriving
    /// the [`Field`] (nullable) from the offset width.
    pub fn from_values<B: AsRef<[u8]>>(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<B>>,
    ) -> Self
    where
        GenericBinaryArray<O>: FromIterator<Option<B>>,
    {
        let values: GenericBinaryArray<O> = values.into_iter().collect();
        let afield = arrow_schema::Field::new(name, values.data_type().clone(), true);
        BinarySerie::from_parts(Field::from_arrow(&afield), values)
    }

    /// The value at `index` as borrowed bytes, or `None` when null / out of bounds —
    /// the zero-copy companion to [`get`](TypedSerie::get).
    pub fn bytes_value(&self, index: usize) -> Option<&[u8]> {
        if index < self.values.len() && self.values.is_valid(index) {
            Some(self.values.value(index))
        } else {
            None
        }
    }

    /// The typed backing [`GenericBinaryArray<O>`](arrow_array::GenericBinaryArray).
    pub fn values(&self) -> &GenericBinaryArray<O> {
        &self.values
    }
}

impl<O: OffsetSizeTrait> Clone for BinarySerie<O> {
    fn clone(&self) -> Self {
        BinarySerie {
            field: self.field.clone(),
            values: self.values.clone(),
        }
    }
}

impl<O: OffsetSizeTrait> fmt::Debug for BinarySerie<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BinarySerie")
            .field("field", &self.field)
            .field("values", &self.values)
            .finish()
    }
}

impl<O: OffsetSizeTrait> Serie for BinarySerie<O> {
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
}

impl<O: OffsetSizeTrait> TypedSerie<Vec<u8>> for BinarySerie<O> {
    fn get(&self, index: usize) -> Option<Vec<u8>> {
        self.bytes_value(index).map(<[u8]>::to_vec)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<Vec<u8>>> + '_> {
        Box::new(self.values.iter().map(|opt| opt.map(<[u8]>::to_vec)))
    }
}
