//! The [`Serie`] scalar of the [`SerieType`](yggdryl_dtype::SerieType) data type.

use std::marker::PhantomData;

use crate::{Scalar, ScalarFactory, TypedScalar};
use arrow_array::ArrayRef;
// The `Serie` base trait is imported anonymously (its `value_type()` accessor is
// all we need) so it does not clash with this module's own `Serie` scalar type.
use yggdryl_dtype::Serie as _;
use yggdryl_dtype::{DataError, DataType, SerieType};

/// A single, possibly-null `list` value: *our array* — a sequence of elements of
/// the value type `D`, backed by one zero-copy Arrow child array.
///
/// The elements live in an [`ArrayRef`] (Arrow's FFI-ready, reference-counted
/// buffers), so [`to_arrow`](Scalar::to_arrow) and [`from_arrow`](Scalar::from_arrow)
/// are reference-count bumps, never element copies; building from inner scalars pays
/// the assembly once, at construction. [`Value`](Scalar::Value) is the backing `dyn
/// Array`, and the *scalar accessors* read elements back out:
/// [`get_scalar_at`](Serie::get_scalar_at) redirects one element through the inner
/// scalar's own `from_arrow`, [`get_at`](Serie::get_at) hands back an element's value
/// as any native Rust target, and [`len`](Serie::len) / [`is_empty`](Serie::is_empty)
/// describe the sequence. (For `int64` there is the concrete, buffer-backed
/// [`Int64Serie`](crate::Int64Serie).)
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_scalar::{Int64Scalar, Scalar, Serie};
///
/// let numbers = Serie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.len(), 2);
/// assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));
/// assert_eq!(numbers.get_scalar_at(1), Some(Int64Scalar::null()));
/// assert_eq!(numbers.get_scalar_at(2), None); // out of bounds
/// assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1); // the native value, any target
/// assert!(numbers.get_at::<i64>(1).is_err()); // a null element holds no value
/// assert_eq!(numbers.data_type().name(), "list");
///
/// // The Arrow round trip shares the buffers — no element is copied.
/// let arrow = numbers.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
///
/// let missing: Serie<Int64Type, Int64Scalar> = Serie::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug)]
pub struct Serie<D, S> {
    data_type: SerieType<D>,
    values: Option<ArrayRef>,
    element: PhantomData<S>,
}

impl<D: DataType + Default, S: Scalar<DataType = D>> Serie<D, S> {
    /// A scalar holding the sequence `values`, assembled once into one Arrow child
    /// array (an empty sequence is the empty serie, not null).
    pub fn new(values: Vec<S>) -> Self {
        // The element type is only needed to type an *empty* child, so it is built
        // lazily — a non-empty serie never constructs `D::default()`.
        Self::from_elements(crate::scalar::concat_scalar_arrays(
            values.iter().map(Scalar::to_arrow).collect(),
            || D::default().to_arrow(),
        ))
    }

    /// The null serie scalar.
    pub fn null() -> Self {
        Self {
            data_type: SerieType::default(),
            values: None,
            element: PhantomData,
        }
    }

    /// A scalar over an already-built Arrow `elements` array, shared zero-copy.
    pub(crate) fn from_elements(elements: ArrayRef) -> Self {
        Self {
            data_type: SerieType::default(),
            values: Some(elements),
            element: PhantomData,
        }
    }

    /// The number of elements, `0` when null or empty ([`is_null`](Scalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.values
            .as_ref()
            .map_or(0, |values| arrow_array::Array::len(values.as_ref()))
    }

    /// The elements converted out as the backing Arrow child [`ArrayRef`] (a
    /// reference-count bump, not a copy), or `None` when the serie is null — the
    /// explicit conversion name, next to [`to_arrow`](Scalar::to_arrow) (the
    /// one-element serie scalar form this array is the child of).
    pub fn to_arrow_array(&self) -> Option<ArrayRef> {
        self.values.clone()
    }

    /// Whether the sequence holds no elements (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The element at `index` as an inner scalar (a null element is the inner null
    /// scalar), or `None` when the serie is null or `index` is out of bounds.
    pub fn get_scalar_at(&self, index: usize) -> Option<S> {
        let values = self.values.as_ref()?;
        if index >= arrow_array::Array::len(values.as_ref()) {
            return None;
        }
        let element = arrow_array::Array::slice(values.as_ref(), index, 1);
        S::from_arrow(element.as_ref()).ok()
    }

    /// The element at `index` read as the native Rust type `T` — the generic
    /// native accessor: the type parameter picks the target and the element
    /// answers through its own `as_*` contract via [`FromScalar`](crate::FromScalar),
    /// so an `int64` element reads as `i64` (or any exactly-representable target)
    /// and a `binary` element as `Vec<u8>` or a `yggdryl-core` `ByteBufferSlice`.
    ///
    /// A null serie errors with [`DataError::NullValue`], an index past the end
    /// with [`DataError::OutOfBounds`], and a null or non-representable element
    /// with the `as_*` contract's own errors.
    pub fn get_at<T: crate::FromScalar>(&self, index: usize) -> Result<T, DataError> {
        let values = self.values.as_ref().ok_or(DataError::NullValue)?;
        let length = arrow_array::Array::len(values.as_ref());
        if index >= length {
            return Err(DataError::OutOfBounds { index, len: length });
        }
        let element = arrow_array::Array::slice(values.as_ref(), index, 1);
        let scalar = S::from_arrow(element.as_ref())?;
        T::from_scalar(&scalar)
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> Default for Serie<D, S> {
    /// The default serie scalar: the empty serie.
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<D: Clone, S> Clone for Serie<D, S> {
    // Cloning bumps the child array's reference count — no element is copied.
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            values: self.values.clone(),
            element: PhantomData,
        }
    }
}

impl<D, S> PartialEq for Serie<D, S> {
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

impl<D, S> Eq for Serie<D, S> {}

impl<D: DataType + Default, S: Scalar<DataType = D>> From<Vec<S>> for Serie<D, S> {
    /// A scalar holding the sequence `values`.
    fn from(values: Vec<S>) -> Self {
        Self::new(values)
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> From<Option<Vec<S>>> for Serie<D, S> {
    /// A scalar holding the sequence, or the null scalar for `None`.
    fn from(values: Option<Vec<S>>) -> Self {
        match values {
            Some(values) => Self::new(values),
            None => Self::null(),
        }
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> Scalar for Serie<D, S> {
    type DataType = SerieType<D>;
    type Value = dyn arrow_array::Array;

    fn data_type(&self) -> &SerieType<D> {
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
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The child array is shared into the one-element serie — a reference-count
        // bump, not a copy.
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([arrow_array::Array::len(values.as_ref())]),
            values.clone(),
            None,
        )
        .expect("a one-element serie of the value type's child is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout and redirects the item child to `D`;
        // the elements themselves are shared zero-copy.
        let data_type = SerieType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a serie data type is a serie array");
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

impl<D: DataType + Default, S: Scalar<DataType = D>>
    TypedScalar<SerieType<D>, dyn arrow_array::Array> for Serie<D, S>
{
}

impl<T, D> ScalarFactory<Vec<T>> for SerieType<D>
where
    D: ScalarFactory<T> + Default,
    D::Scalar: Scalar<DataType = D>,
{
    type Scalar = Serie<D, D::Scalar>;

    /// A serie scalar holding the native `values`, each converted through the value
    /// type's own scalar factory.
    fn scalar(&self, values: Vec<T>) -> Self::Scalar {
        Serie::new(
            values
                .into_iter()
                .map(|value| self.value_type().scalar(value))
                .collect(),
        )
    }

    /// The null serie scalar.
    fn null_scalar(&self) -> Self::Scalar {
        Serie::null()
    }

    /// The default serie scalar: the empty serie.
    fn default_scalar(&self) -> Self::Scalar {
        Serie::new(Vec::new())
    }
}
