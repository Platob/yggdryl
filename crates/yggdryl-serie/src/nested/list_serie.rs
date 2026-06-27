//! [`ListSerie<O>`] — a list column backed by an Arrow `GenericListArray`. Its element
//! values are a single flattened child [`Serie`] (built recursively); a per-row
//! [`value_slice`](ListSerie::value_slice) returns the sub-list as a zero-copy column.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, GenericListArray, OffsetSizeTrait};
use arrow_schema::DataType as ADataType;
use yggdryl_schema::Field;

use crate::error::SerieResult;
use crate::nested::NestedSerie;
use crate::scalar::Scalar;
use crate::serie::{dispatch, Serie, SerieRef};

/// A list column. `O = i32` is `List`, `O = i64` is `LargeList`. The flattened element
/// values are a child [`Serie`]; each row is a sub-slice of it.
#[derive(Debug, Clone)]
pub struct ListSerie<O: OffsetSizeTrait> {
    field: Field,
    array: GenericListArray<O>,
    values: SerieRef,
}

impl<O: OffsetSizeTrait> ListSerie<O> {
    /// Wraps a field and a matching list array, building the flattened element column
    /// **recursively**. Used by the [factory](crate::from_arrow).
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> SerieResult<ListSerie<O>> {
        let array = array
            .as_any()
            .downcast_ref::<GenericListArray<O>>()
            .expect("data type matched the list array")
            .clone();
        let item = match array.data_type() {
            ADataType::List(f) | ADataType::LargeList(f) => Field::from_arrow(f.as_ref()),
            other => Field::from_arrow(&arrow_schema::Field::new("item", other.clone(), true)),
        };
        let values = dispatch(item, array.values().clone())?;
        Ok(ListSerie {
            field,
            array,
            values,
        })
    }

    /// The flattened element column (all rows' elements concatenated).
    pub fn values(&self) -> &SerieRef {
        &self.values
    }

    /// The sub-list at `index` as a zero-copy [`Serie`], or `None` when null / out of
    /// bounds.
    pub fn value_slice(&self, index: usize) -> Option<SerieRef> {
        if self.is_null(index) {
            return None;
        }
        let offsets = self.array.value_offsets();
        let start = offsets[index].as_usize();
        let end = offsets[index + 1].as_usize();
        Some(self.values.slice(start, end - start))
    }
}

impl<O: OffsetSizeTrait> Serie for ListSerie<O> {
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

    /// A readable `[v0, v1, …]` rendering of the sub-list at `index`.
    fn value_at(&self, index: usize) -> Scalar {
        if self.is_null(index) {
            return Scalar::Null;
        }
        let offsets = self.array.value_offsets();
        let (start, end) = (offsets[index].as_usize(), offsets[index + 1].as_usize());
        let mut text = String::from("[");
        for k in start..end {
            if k > start {
                text.push_str(", ");
            }
            text.push_str(&self.values.value_at(k).to_string());
        }
        text.push(']');
        Scalar::Other(text)
    }
}

impl<O: OffsetSizeTrait> NestedSerie for ListSerie<O> {
    fn child_count(&self) -> usize {
        1
    }

    fn child(&self, index: usize) -> Option<SerieRef> {
        (index == 0).then(|| self.values.clone())
    }
}
