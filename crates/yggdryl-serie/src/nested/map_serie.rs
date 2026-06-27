//! [`MapSerie`] — a map column backed by an Arrow `MapArray`. Its keys and values are
//! two flattened child [`Serie`]s (built recursively); each row is a run of key/value
//! pairs delimited by the map offsets.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, MapArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::Field as AField;
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::nested::NestedSerie;
use crate::scalar::Scalar;
use crate::serie::{from_array, Serie, SerieRef};

/// A map column: a flattened `keys` column and a flattened `values` column, with each
/// row a `[offset, next_offset)` run of pairs.
#[derive(Debug, Clone)]
pub struct MapSerie {
    field: Field,
    array: MapArray,
    keys: SerieRef,
    values: SerieRef,
}

impl MapSerie {
    /// Wraps a field and a matching `MapArray`, building the flattened key and value
    /// columns **recursively**. Used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> SerieResult<MapSerie> {
        let array = array
            .as_any()
            .downcast_ref::<MapArray>()
            .expect("data type matched the map array")
            .clone();
        let keys = from_array("key", array.keys().clone())?;
        let values = from_array("value", array.values().clone())?;
        Ok(MapSerie {
            field,
            array,
            keys,
            values,
        })
    }

    /// Builds a map column named `name` from its **flattened** key and value columns and
    /// the per-row entry counts — the one-line constructor the bindings build a map from a
    /// list-of-dicts with. Row `i` takes the next `lengths[i]` pairs off `keys`/`values`;
    /// a `None` length marks a **null** row. The summed lengths must equal each child's
    /// length (the keys and values must be equal-length). Keys must be non-null.
    ///
    /// ```
    /// use yggdryl_serie::{Int32Serie, MapSerie, Serie, SerieRef, VarcharSerie};
    /// use std::sync::Arc;
    ///
    /// // [{"a": 1, "b": 2}, {"c": 3}] from the flat keys/values
    /// let keys: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("key", vec![Some("a"), Some("b"), Some("c")]));
    /// let vals: SerieRef = Arc::new(Int32Serie::from_values("value", vec![Some(1), Some(2), Some(3)]));
    /// let map = MapSerie::from_values("m", keys, vals, &[Some(2), Some(1)]).unwrap();
    /// assert_eq!(map.len(), 2);
    /// assert_eq!(map.value_at(0).to_string(), "{a=1, b=2}");
    /// ```
    pub fn from_values(
        name: impl Into<String>,
        keys: SerieRef,
        values: SerieRef,
        lengths: &[Option<usize>],
    ) -> SerieResult<MapSerie> {
        if keys.len() != values.len() {
            return Err(SerieError::Arrow(format!(
                "map keys have {} rows but values have {}",
                keys.len(),
                values.len()
            )));
        }
        let total: usize = lengths.iter().map(|l| l.unwrap_or(0)).sum();
        if total != keys.len() {
            return Err(SerieError::Arrow(format!(
                "map lengths sum to {total} but the flattened entries have {} rows",
                keys.len()
            )));
        }
        // The entries struct's key field must be non-nullable (Arrow's map invariant).
        let key_field = keys
            .field()
            .copy(Some("key".to_string()), None, Some(false), None);
        let value_field = values
            .field()
            .copy(Some("value".to_string()), None, None, None);
        let key_arrow = Arc::new(key_field.to_arrow()?);
        let value_arrow = Arc::new(value_field.to_arrow()?);
        let entries = StructArray::try_new(
            vec![key_arrow, value_arrow].into(),
            vec![keys.array(), values.array()],
            None,
        )
        .map_err(|e| SerieError::Arrow(e.to_string()))?;
        let entries_field = Arc::new(AField::new("entries", entries.data_type().clone(), false));
        let offsets = OffsetBuffer::<i32>::from_lengths(lengths.iter().map(|l| l.unwrap_or(0)));
        let nulls = lengths
            .iter()
            .any(|l| l.is_none())
            .then(|| NullBuffer::from(lengths.iter().map(|l| l.is_some()).collect::<Vec<_>>()));
        let array = MapArray::try_new(entries_field, offsets, entries, nulls, false)
            .map_err(|e| SerieError::Arrow(e.to_string()))?;
        let dtype = DataType::map(
            key_field.data_type().clone(),
            value_field.data_type().clone(),
            false,
        );
        MapSerie::from_parts(Field::new(name, dtype, true), Arc::new(array))
    }

    /// The flattened key column.
    pub fn keys(&self) -> &SerieRef {
        &self.keys
    }

    /// The flattened value column.
    pub fn values(&self) -> &SerieRef {
        &self.values
    }
}

impl Serie for MapSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        Arc::new(self.array.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.array.len()
    }

    fn null_count(&self) -> usize {
        self.array.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.array.len() || self.array.is_null(index)
    }

    fn as_nested(&self) -> Option<&dyn NestedSerie> {
        Some(self)
    }

    /// A readable `{key=value, …}` rendering of the entries at `index`.
    fn value_at(&self, index: usize) -> Scalar {
        if self.is_null(index) {
            return Scalar::Null;
        }
        let offsets = self.array.value_offsets();
        let (start, end) = (offsets[index] as usize, offsets[index + 1] as usize);
        let mut text = String::from("{");
        for k in start..end {
            if k > start {
                text.push_str(", ");
            }
            text.push_str(&self.keys.value_at(k).to_string());
            text.push('=');
            text.push_str(&self.values.value_at(k).to_string());
        }
        text.push('}');
        Scalar::Other(text)
    }
}

impl NestedSerie for MapSerie {
    fn child_count(&self) -> usize {
        2
    }

    fn child(&self, index: usize) -> Option<SerieRef> {
        match index {
            0 => Some(self.keys.clone()),
            1 => Some(self.values.clone()),
            _ => None,
        }
    }
}
