//! [`PrimitiveSerie<A>`] — the column backing every fixed-width Arrow scalar: the
//! integers, floats, decimals and the temporal physical types. Parameterised by the
//! Arrow [`ArrowPrimitiveType`](arrow_array::types::ArrowPrimitiveType) marker, so one
//! type covers them all; named [aliases](crate) (`Int32Serie`, `Float64Serie`,
//! `TimestampMicrosecondSerie`, …) pin the common ones.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow_array::types::ArrowPrimitiveType;
use arrow_array::{Array, ArrayRef, PrimitiveArray};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::serie::{Serie, TypedSerie};

/// A column of a fixed-width Arrow scalar type `A` (an integer, float, decimal or a
/// temporal physical type), pairing a [`Field`] with an Arrow
/// [`PrimitiveArray<A>`](arrow_array::PrimitiveArray).
pub struct PrimitiveSerie<A: ArrowPrimitiveType> {
    field: Field,
    values: PrimitiveArray<A>,
}

impl<A: ArrowPrimitiveType> PrimitiveSerie<A> {
    /// Wraps a field and an already-matching array (no validation) — the path the
    /// [factory](crate::from_arrow) uses after it has checked the types.
    pub(crate) fn from_parts(field: Field, values: PrimitiveArray<A>) -> Self {
        PrimitiveSerie { field, values }
    }

    /// Builds from a [`Field`] and an Arrow array, checking the field's
    /// [`DataType`](yggdryl_schema::DataType) maps to the array's Arrow type.
    pub fn new(field: Field, values: PrimitiveArray<A>) -> SerieResult<Self> {
        let expected = field.data_type().to_arrow()?;
        if &expected != values.data_type() {
            return Err(SerieError::TypeMismatch {
                expected: expected.to_string(),
                found: values.data_type().to_string(),
            });
        }
        Ok(PrimitiveSerie::from_parts(field, values))
    }

    /// Builds a column named `name` from an iterator of optional values, deriving the
    /// [`Field`] (nullable) from the Arrow type of `A`.
    pub fn from_values(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<A::Native>>,
    ) -> Self
    where
        PrimitiveArray<A>: FromIterator<Option<A::Native>>,
    {
        let values: PrimitiveArray<A> = values.into_iter().collect();
        let afield = arrow_schema::Field::new(name, values.data_type().clone(), true);
        PrimitiveSerie::from_parts(Field::from_arrow(&afield), values)
    }

    /// The typed backing [`PrimitiveArray<A>`](arrow_array::PrimitiveArray).
    pub fn values(&self) -> &PrimitiveArray<A> {
        &self.values
    }
}

impl<A: ArrowPrimitiveType> Clone for PrimitiveSerie<A> {
    fn clone(&self) -> Self {
        PrimitiveSerie {
            field: self.field.clone(),
            values: self.values.clone(),
        }
    }
}

impl<A: ArrowPrimitiveType> fmt::Debug for PrimitiveSerie<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrimitiveSerie")
            .field("field", &self.field)
            .field("values", &self.values)
            .finish()
    }
}

impl<A: ArrowPrimitiveType> Serie for PrimitiveSerie<A> {
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

impl<A: ArrowPrimitiveType> TypedSerie<A::Native> for PrimitiveSerie<A> {
    fn get(&self, index: usize) -> Option<A::Native> {
        if index < self.values.len() && self.values.is_valid(index) {
            Some(self.values.value(index))
        } else {
            None
        }
    }

    fn iter(&self) -> Box<dyn Iterator<Item = Option<A::Native>> + '_> {
        Box::new(self.values.iter())
    }
}
