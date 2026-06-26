//! [`ArrayColumn`] — the materialized [`Column`] backing: an Arrow [`ArrayRef`]
//! paired with its [`Field`]. Slicing is zero-copy (it reuses the Arrow buffers)
//! and casting goes through Arrow's cast kernel. Gated behind the `dataframe`
//! feature.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::{Column, ColumnError, DataType, Field};

use arrow_array::ArrayRef;

/// A single column whose values live in an Arrow array — the column a
/// [`DataFrame`](crate::DataFrame) hands back from
/// [`column`](crate::Frame::column). It is always [`materialized`](Column::is_materialized)
/// (its [`len`](Column::len) is known); `rename` / `cast` / `slice` reuse the
/// Arrow buffers where they can.
///
/// ```
/// use std::sync::Arc;
/// use arrow_array::Int64Array;
/// use yggdryl_saga::{ArrayColumn, Column, Field, PrimitiveType};
///
/// let col = ArrayColumn::new(
///     Field::new("px", PrimitiveType::Int64.into(), false),
///     Arc::new(Int64Array::from(vec![10, 20, 30])),
/// );
/// assert_eq!(col.name(), "px");
/// assert_eq!(col.len(), Some(3));
/// assert_eq!(col.head(2).unwrap().len(), Some(2));
/// ```
#[derive(Clone)]
pub struct ArrayColumn {
    field: Field,
    array: ArrayRef,
}

impl ArrayColumn {
    /// Pairs a [`Field`] with its Arrow array. The array's length is the column's
    /// length; its Arrow type should match the field's [`DataType`].
    pub fn new(field: Field, array: ArrayRef) -> ArrayColumn {
        ArrayColumn { field, array }
    }

    /// Borrows the underlying Arrow array.
    pub fn array(&self) -> &ArrayRef {
        &self.array
    }

    /// Consumes the column, returning its Arrow array.
    pub fn into_array(self) -> ArrayRef {
        self.array
    }
}

impl Column for ArrayColumn {
    fn field(&self) -> &Field {
        &self.field
    }

    fn is_materialized(&self) -> bool {
        true
    }

    fn len(&self) -> Option<usize> {
        Some(self.array.len())
    }

    fn rename(mut self, name: impl Into<String>) -> ArrayColumn {
        self.field = self.field.with_name(name);
        self
    }

    fn cast(self, data_type: DataType) -> Result<ArrayColumn, ColumnError> {
        log_event!(
            debug,
            "ArrayColumn::cast {} -> {data_type}",
            self.field.name()
        );
        let target = data_type.to_arrow();
        let array = arrow_cast::cast(&self.array, &target).map_err(|_| ColumnError::Cast {
            from: self.field.data_type().clone(),
            to: data_type.clone(),
        })?;
        Ok(ArrayColumn {
            field: self.field.with_data_type(data_type),
            array,
        })
    }

    fn slice(self, offset: usize, length: usize) -> Result<ArrayColumn, ColumnError> {
        let len = self.array.len();
        let offset = offset.min(len);
        let length = length.min(len - offset);
        Ok(ArrayColumn {
            field: self.field,
            array: self.array.slice(offset, length),
        })
    }

    fn tail(self, n: usize) -> Result<ArrayColumn, ColumnError> {
        let len = self.array.len();
        let n = n.min(len);
        self.slice(len - n, n)
    }
}

impl fmt::Debug for ArrayColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ArrayColumn {{ {}, len: {} }}",
            self.field.to_str(),
            self.array.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrimitiveType;
    use arrow_array::{Int64Array, StringArray};
    use std::sync::Arc;

    fn col() -> ArrayColumn {
        ArrayColumn::new(
            Field::new("px", PrimitiveType::Int64.into(), false),
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
        )
    }

    #[test]
    fn identity_and_length() {
        let c = col();
        assert_eq!(c.name(), "px");
        assert_eq!(c.data_type(), &DataType::from(PrimitiveType::Int64));
        assert!(c.is_materialized());
        assert_eq!(c.len(), Some(4));
        // A materialized column has no holder frame by default (detached).
        assert!(c.frame().is_none());
    }

    #[test]
    fn slice_is_zero_copy_and_clamped() {
        assert_eq!(col().head(2).unwrap().len(), Some(2));
        assert_eq!(col().tail(1).unwrap().len(), Some(1));
        assert_eq!(col().slice(10, 5).unwrap().len(), Some(0));
    }

    #[test]
    fn cast_converts_the_array() {
        let c = col().cast(PrimitiveType::Utf8.into()).unwrap();
        assert_eq!(c.data_type(), &DataType::from(PrimitiveType::Utf8));
        let strs = c.array().as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(strs.value(0), "1");
    }

    #[test]
    fn rename_keeps_data() {
        let c = col().rename("price");
        assert_eq!(c.name(), "price");
        assert_eq!(c.len(), Some(4));
    }
}
