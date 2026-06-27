//! [`MapSerie`] — a map column backed by an Arrow `MapArray`. Its keys and values are
//! two flattened child [`Serie`]s (built recursively); each row is a run of key/value
//! pairs delimited by the map offsets.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, MapArray};
use yggdryl_schema::Field;

use crate::error::SerieResult;
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
