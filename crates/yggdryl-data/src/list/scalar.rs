//! The [`ListScalar`] scalar of the [`ListType`](super::ListType) data type.

use std::marker::PhantomData;

use super::ListType;
use crate::{DataError, RawDataType, RawScalar, Scalar};
use arrow_array::ArrayRef;

/// A single, possibly-null `list` value: *our array* — a sequence of elements of
/// the value type `D`, backed by one zero-copy Arrow child array.
///
/// The elements live in an [`ArrayRef`] (Arrow's FFI-ready, reference-counted
/// buffers), so [`to_arrow`](RawScalar::to_arrow) and
/// [`from_arrow`](RawScalar::from_arrow) are reference-count bumps, never element
/// copies; building from inner scalars pays the assembly once, at construction.
/// [`Value`](RawScalar::Value) is the backing `dyn Array`, and the *scalar
/// accessors* read elements back out: [`get_scalar_at`](ListScalar::get_scalar_at)
/// redirects one element through the inner scalar's own `from_arrow`,
/// [`len`](ListScalar::len) / [`is_empty`](ListScalar::is_empty) describe the
/// sequence. (For `int64` there is the concrete, buffer-backed
/// [`Int64Array`](crate::Int64Array).)
///
/// ```
/// use yggdryl_data::{Int64, Int64Scalar, ListScalar, RawDataType, RawScalar};
///
/// let numbers = ListScalar::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.len(), 2);
/// assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));
/// assert_eq!(numbers.get_scalar_at(1), Some(Int64Scalar::null()));
/// assert_eq!(numbers.get_scalar_at(2), None); // out of bounds
/// assert_eq!(numbers.data_type().name(), "list");
///
/// // The Arrow round trip shares the buffers — no element is copied.
/// let arrow = numbers.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(ListScalar::from_arrow(arrow.as_ref()).unwrap(), numbers);
///
/// let missing: ListScalar<Int64, Int64Scalar> = ListScalar::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug)]
pub struct ListScalar<D, S> {
    data_type: ListType<D>,
    values: Option<ArrayRef>,
    element: PhantomData<S>,
}

impl<D: RawDataType + Default, S: RawScalar<D>> ListScalar<D, S> {
    /// A scalar holding the sequence `values`, assembled once into one Arrow child
    /// array (an empty sequence is the empty list, not null).
    pub fn new(values: Vec<S>) -> Self {
        let item_type = D::default().to_arrow();
        Self::from_elements(crate::raw_scalar::concat_scalar_arrays(
            values.iter().map(RawScalar::to_arrow).collect(),
            &item_type,
        ))
    }

    /// The null list scalar.
    pub fn null() -> Self {
        Self {
            data_type: ListType::default(),
            values: None,
            element: PhantomData,
        }
    }

    /// A scalar over an already-built Arrow `elements` array, shared zero-copy.
    pub(crate) fn from_elements(elements: ArrayRef) -> Self {
        Self {
            data_type: ListType::default(),
            values: Some(elements),
            element: PhantomData,
        }
    }

    /// The number of elements, `0` when null or empty ([`is_null`](RawScalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.values
            .as_ref()
            .map_or(0, |values| arrow_array::Array::len(values.as_ref()))
    }

    /// Whether the sequence holds no elements (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The element at `index` as an inner scalar (a null element is the inner null
    /// scalar), or `None` when the list is null or `index` is out of bounds.
    pub fn get_scalar_at(&self, index: usize) -> Option<S> {
        let values = self.values.as_ref()?;
        if index >= arrow_array::Array::len(values.as_ref()) {
            return None;
        }
        let element = arrow_array::Array::slice(values.as_ref(), index, 1);
        S::from_arrow(element.as_ref()).ok()
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Default for ListScalar<D, S> {
    /// The default list scalar: the empty list.
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<D: Clone, S> Clone for ListScalar<D, S> {
    // Cloning bumps the child array's reference count — no element is copied.
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            values: self.values.clone(),
            element: PhantomData,
        }
    }
}

impl<D, S> PartialEq for ListScalar<D, S> {
    // The backing arrays compare by value through `dyn Array` equality, so two
    // lists are equal when their elements (and nulls) are.
    fn eq(&self, other: &Self) -> bool {
        match (&self.values, &other.values) {
            (None, None) => true,
            (Some(left), Some(right)) => left.as_ref() == right.as_ref(),
            _ => false,
        }
    }
}

impl<D, S> Eq for ListScalar<D, S> {}

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
    type Value = dyn arrow_array::Array;

    fn data_type(&self) -> &ListType<D> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&(dyn arrow_array::Array + 'static)> {
        self.values.as_deref()
    }

    fn to_arrow(&self) -> ArrayRef {
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&crate::RawDataType::to_arrow(&self.data_type), 1);
        };
        // The child array is shared into the one-element list — a reference-count
        // bump, not a copy.
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([arrow_array::Array::len(values.as_ref())]),
            values.clone(),
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
        // the elements themselves are shared zero-copy.
        let data_type = ListType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a list data type is a list array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(array.value(0))
        };
        Ok(Self {
            data_type,
            values,
            element: PhantomData,
        })
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Scalar<dyn arrow_array::Array>
    for ListScalar<D, S>
{
    type Type = ListType<D>;
}
