//! [`CategoricalSerie`] — a **dictionary-encoded** column for *repeated values*: it
//! stores the distinct values once (the dictionary) plus a compact per-row integer
//! **code**, so a column with few distinct values is held compactly. It is a lazy view
//! (`is_materialized()` is `false`); [`materialize`](crate::Serie::materialize) decodes
//! it back into a real, flat [`Serie`].

use std::any::Any;

use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayRef, DictionaryArray};
use arrow_schema::DataType as ADataType;
use yggdryl_schema::Field;

use crate::scalar::{scalar_at, Scalar};
use crate::serie::{from_arrow, Serie, SerieRef};
use crate::SerieResult;

/// A dictionary-encoded column: an `int32` code per row into a dictionary of the
/// distinct values. Compact for repeated values; decodes to a flat column on
/// [`materialize`](crate::Serie::materialize).
#[derive(Debug, Clone)]
pub struct CategoricalSerie {
    field: Field,
    dict: DictionaryArray<Int32Type>,
}

impl CategoricalSerie {
    /// Dictionary-encodes `serie` (its distinct values become the dictionary). Cheap
    /// when the column has many repeats.
    pub fn from_serie(serie: &dyn Serie) -> SerieResult<CategoricalSerie> {
        let value_type = serie.data_type().to_arrow()?;
        let dict_type = ADataType::Dictionary(Box::new(ADataType::Int32), Box::new(value_type));
        let encoded = arrow_cast::cast(serie.array().as_ref(), &dict_type)?;
        let dict = encoded
            .as_any()
            .downcast_ref::<DictionaryArray<Int32Type>>()
            .expect("a cast to Dictionary yields a DictionaryArray")
            .clone();
        Ok(CategoricalSerie {
            field: serie.field().clone(),
            dict,
        })
    }

    /// The number of distinct categories.
    pub fn category_count(&self) -> usize {
        self.dict.values().len()
    }

    /// The distinct values (the dictionary) as a [`Serie`] named `"categories"`.
    pub fn categories(&self) -> SerieResult<SerieRef> {
        from_arrow(
            self.field.copy(Some("categories".into()), None, None, None),
            self.dict.values().clone(),
        )
    }

    /// The dictionary **code** at row `index`, or `None` when null / out of bounds.
    pub fn code_at(&self, index: usize) -> Option<i32> {
        let keys = self.dict.keys();
        (index < keys.len() && keys.is_valid(index)).then(|| keys.value(index))
    }

    /// The value of the category with the given `code`, or [`Null`](Scalar::Null) if out
    /// of range.
    pub fn category(&self, code: usize) -> Scalar {
        scalar_at(self.dict.values(), code)
    }
}

impl Serie for CategoricalSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        // Decode (materialise) the dictionary back to its flat value type. The value
        // type matches the field, so this is the array `materialize` rebuilds.
        let value_type = self
            .field
            .data_type()
            .to_arrow()
            .expect("a categorical's value type maps to Arrow");
        arrow_cast::cast(&self.dict, &value_type).expect("a dictionary decodes to its value type")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.dict.len()
    }

    fn null_count(&self) -> usize {
        self.dict.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.dict.len() || self.dict.is_null(index)
    }

    fn is_materialized(&self) -> bool {
        false
    }

    fn value_at(&self, index: usize) -> Scalar {
        match self.code_at(index) {
            Some(code) if code >= 0 => self.category(code as usize),
            _ => Scalar::Null,
        }
    }
}
