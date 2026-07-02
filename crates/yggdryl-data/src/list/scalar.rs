//! The [`ListScalar`] scalar of the [`ListType`](super::ListType) data type.

use super::ListType;
use crate::{DataError, RawDataType, RawScalar, Scalar};

/// A single, possibly-null `list` value: a sequence of inner scalars `S` of the
/// value type `D`.
///
/// Its [`Value`](RawScalar::Value) is the borrowed slice `[S]`, so
/// [`value`](RawScalar::value) yields `Option<&[S]>`. The Arrow form is a
/// one-element `ListArray` whose child concatenates the element scalars' own Arrow
/// forms; [`from_arrow`](RawScalar::from_arrow) redirects every element back
/// through `S::from_arrow`.
///
/// ```
/// use yggdryl_data::{Int64, Int64Scalar, ListScalar, RawDataType, RawScalar};
///
/// let numbers = ListScalar::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.value().map(<[Int64Scalar]>::len), Some(2));
/// assert_eq!(numbers.data_type().name(), "list");
///
/// // The Arrow round trip preserves the elements.
/// let arrow = numbers.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(ListScalar::from_arrow(arrow.as_ref()).unwrap(), numbers);
///
/// let missing: ListScalar<Int64, Int64Scalar> = ListScalar::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListScalar<D, S> {
    data_type: ListType<D>,
    values: Option<Vec<S>>,
}

impl<D: RawDataType + Default, S: RawScalar<D>> ListScalar<D, S> {
    /// A scalar holding the sequence `values` (an empty sequence is the empty
    /// list, not null).
    pub fn new(values: Vec<S>) -> Self {
        Self {
            data_type: ListType::default(),
            values: Some(values),
        }
    }

    /// The null list scalar.
    pub fn null() -> Self {
        Self {
            data_type: ListType::default(),
            values: None,
        }
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Default for ListScalar<D, S> {
    /// The default list scalar: the empty list.
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> From<Vec<S>> for ListScalar<D, S> {
    /// A scalar holding the sequence `values`.
    fn from(values: Vec<S>) -> Self {
        Self::new(values)
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> From<Option<Vec<S>>> for ListScalar<D, S> {
    /// A scalar holding the sequence, or the null scalar for `None`.
    fn from(values: Option<Vec<S>>) -> Self {
        match values {
            Some(values) => Self::new(values),
            None => Self::null(),
        }
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> RawScalar<ListType<D>> for ListScalar<D, S> {
    type Value = [S];

    fn data_type(&self) -> &ListType<D> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&[S]> {
        self.values.as_deref()
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        let item_field = self.data_type.item_field();
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&crate::RawDataType::to_arrow(&self.data_type), 1);
        };
        let child = crate::raw_scalar::concat_scalar_arrays(
            values.iter().map(RawScalar::to_arrow).collect(),
            item_field.data_type(),
        );
        let array = arrow_array::ListArray::try_new(
            item_field,
            arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
            child,
            None,
        )
        .expect("a one-element list of the value type's child is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout and redirects the item child to `D`;
        // then every element redirects to `S`.
        let data_type = ListType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a list data type is a list array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            let elements = array.value(0);
            Some(crate::raw_scalar::scalars_from_elements(elements.as_ref())?)
        };
        Ok(Self { data_type, values })
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Scalar<[S]> for ListScalar<D, S> {
    type Type = ListType<D>;
}
