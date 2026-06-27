//! [`ListSerie<O>`] — a list column backed by an Arrow `GenericListArray`. Its element
//! values are a single flattened child [`Serie`] (built recursively); a per-row
//! [`value_slice`](ListSerie::value_slice) returns the sub-list as a zero-copy column.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, GenericListArray, OffsetSizeTrait};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::DataType as ADataType;
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
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

    /// Builds a list column named `name` from its **flattened** element column and the
    /// per-row element counts — the one-line constructor the bindings build a list from a
    /// list-of-lists with. Row `i` takes the next `lengths[i]` elements off `values`; a
    /// `None` length marks a **null** row (contributing no elements). The summed lengths
    /// must equal `values.len()`.
    ///
    /// ```
    /// use yggdryl_serie::{Int32Serie, ListSerie, NestedSerie, Serie, SerieRef};
    /// use std::sync::Arc;
    ///
    /// // [[1, 2], [], None, [3]] from the flat values [1, 2, 3]
    /// let flat: SerieRef = Arc::new(Int32Serie::from_values("item", vec![Some(1), Some(2), Some(3)]));
    /// let list = ListSerie::<i32>::from_values("nums", flat, &[Some(2), Some(0), None, Some(1)]).unwrap();
    /// assert_eq!(list.len(), 4);
    /// assert_eq!(list.null_count(), 1);
    /// assert_eq!(list.value_slice(0).unwrap().len(), 2);
    /// assert_eq!(list.value_at(3).to_string(), "[3]");
    /// ```
    pub fn from_values(
        name: impl Into<String>,
        values: SerieRef,
        lengths: &[Option<usize>],
    ) -> SerieResult<ListSerie<O>> {
        let total: usize = lengths.iter().map(|l| l.unwrap_or(0)).sum();
        if total != values.len() {
            return Err(SerieError::Arrow(format!(
                "list lengths sum to {total} but the flattened values column has {} rows",
                values.len()
            )));
        }
        // The item field carries the element type; name it `item` (the Arrow convention).
        let item = values
            .field()
            .copy(Some("item".to_string()), None, None, None);
        let item_arrow = Arc::new(item.to_arrow()?);
        let offsets = OffsetBuffer::<O>::from_lengths(lengths.iter().map(|l| l.unwrap_or(0)));
        let nulls = lengths
            .iter()
            .any(|l| l.is_none())
            .then(|| NullBuffer::from(lengths.iter().map(|l| l.is_some()).collect::<Vec<_>>()));
        let array = GenericListArray::<O>::try_new(item_arrow, offsets, values.array(), nulls)
            .map_err(|e| SerieError::Arrow(e.to_string()))?;
        let dtype = if O::IS_LARGE {
            DataType::large_list(item)
        } else {
            DataType::list(item)
        };
        ListSerie::from_parts(Field::new(name, dtype, true), Arc::new(array))
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
