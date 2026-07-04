//! The statically-typed [`TypedSerie`] scalar of the
//! [`TypedSerieType`](yggdryl_dtype::TypedSerieType) data type.

use std::marker::PhantomData;

use crate::{AnySerie, Scalar, ScalarFactory, TypedScalar};
use arrow_array::ArrayRef;
// The `Serie` / `TypedSerie` dtype traits are imported anonymously (their
// `item_field()` / `value_type()` accessors are all we need) so they do not clash
// with this module's own scalar types.
use yggdryl_dtype::{DataError, DataType, TypedSerieType};
use yggdryl_dtype::{Serie as _, TypedSerie as _};

/// A single, possibly-null `list` value: *our array* — a sequence of elements of the
/// value type `D`, backed by one zero-copy Arrow child array.
///
/// It is the statically-typed counterpart of the dynamic [`Serie`](crate::Serie): the
/// elements live in an [`ArrayRef`] (Arrow's FFI-ready, reference-counted buffers), so
/// [`to_arrow_scalar`](Scalar::to_arrow_scalar) and [`from_arrow`](Scalar::from_arrow)
/// are reference-count bumps, never element copies; building from inner scalars pays
/// the assembly once, at construction. [`Value`](Scalar::Value) is the backing `dyn
/// Array`, and the *scalar accessors* read elements back out:
/// [`scalar_at`](TypedSerie::scalar_at) redirects one element through the
/// inner scalar's own `from_arrow`, [`get_at`](TypedSerie::get_at) hands back an
/// element's value as any native Rust target, and [`len`](TypedSerie::len) /
/// [`is_empty`](TypedSerie::is_empty) describe the sequence.
/// [`erase`](TypedSerie::erase) drops the static element type to a dynamic
/// [`Serie`](crate::Serie). (For `int64` there is the concrete, buffer-backed
/// [`Int64Serie`](crate::Int64Serie).)
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedSerie};
///
/// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.len(), 2);
/// assert_eq!(numbers.scalar_at(0), Some(Int64Scalar::new(1)));
/// assert_eq!(numbers.scalar_at(1), Some(Int64Scalar::null()));
/// assert_eq!(numbers.scalar_at(2), None); // out of bounds
/// assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1); // the native value, any target
/// assert!(numbers.get_at::<i64>(1).is_err()); // a null element holds no value
/// assert_eq!(numbers.data_type().name(), "list");
///
/// // The Arrow round trip shares the buffers — no element is copied.
/// let arrow = numbers.to_arrow_scalar();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(TypedSerie::from_arrow(arrow.as_ref()).unwrap(), numbers);
///
/// let missing: TypedSerie<Int64Type, Int64Scalar> = TypedSerie::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug)]
pub struct TypedSerie<D, S> {
    data_type: TypedSerieType<D>,
    values: Option<AnySerie>,
    element: PhantomData<S>,
}

impl<D: DataType + Default, S: Scalar<DataType = D>> TypedSerie<D, S> {
    /// A scalar holding the sequence `values`, assembled once into one Arrow child
    /// array (an empty sequence is the empty serie, not null).
    pub fn new(values: Vec<S>) -> Self {
        // The element type is only needed to type an *empty* child, so it is built
        // lazily — a non-empty serie never constructs `D::default()`.
        Self::from_elements(crate::scalar::concat_scalar_arrays(
            values.iter().map(Scalar::to_arrow_scalar).collect(),
            || D::default().to_arrow(),
        ))
    }

    /// The null serie scalar.
    pub fn null() -> Self {
        Self {
            data_type: TypedSerieType::default(),
            values: None,
            element: PhantomData,
        }
    }

    /// A scalar over an already-built Arrow `elements` array, decomposed into the
    /// crate's own serie and shared zero-copy.
    pub(crate) fn from_elements(elements: ArrayRef) -> Self {
        Self {
            data_type: TypedSerieType::default(),
            values: Some(AnySerie::from_arrow(elements)),
            element: PhantomData,
        }
    }

    /// Drop the static element type, returning the dynamic [`Serie`](crate::Serie)
    /// over the same shared child array (a reference-count bump, not a copy).
    pub fn erase(&self) -> crate::Serie {
        crate::Serie::from_parts(self.data_type.erase(), self.values.clone())
    }

    /// The number of elements, `0` when null or empty ([`is_null`](Scalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.values.as_ref().map_or(0, AnySerie::len)
    }

    /// Whether the sequence holds no elements (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The element at `index` as an inner scalar (a null element is the inner null
    /// scalar), or `None` when the serie is null or `index` is out of bounds.
    pub fn scalar_at(&self, index: usize) -> Option<S> {
        let values = self.values.as_ref()?;
        if index >= values.len() {
            return None;
        }
        let element = values.to_arrow().slice(index, 1);
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
        let length = values.len();
        if index >= length {
            return Err(DataError::OutOfBounds { index, len: length });
        }
        let element = values.to_arrow().slice(index, 1);
        let scalar = S::from_arrow(element.as_ref())?;
        T::from_scalar(&scalar)
    }

    /// An iterator over the elements as inner scalars, in order (a null element is
    /// the inner null scalar; a null serie yields nothing). The element column is
    /// reconstituted **once**, and each step slices one element from it — so the
    /// whole walk is linear, unlike a [`scalar_at`](TypedSerie::scalar_at)
    /// loop, which reconstitutes the column on every call. The returned iterator
    /// owns a reference-counted view of the column, so it borrows nothing and is
    /// [`ExactSizeIterator`] / [`DoubleEndedIterator`].
    ///
    /// ```
    /// use yggdryl_scalar::{Int64Scalar, Scalar, TypedSerie};
    ///
    /// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
    /// let scalars: Vec<Int64Scalar> = numbers.iter_scalars().collect();
    /// assert_eq!(scalars, vec![Int64Scalar::new(1), Int64Scalar::null()]);
    /// assert_eq!(numbers.iter_scalars().len(), 2); // exact size, no walk
    /// assert_eq!(TypedSerie::<_, Int64Scalar>::null().iter_scalars().count(), 0);
    /// ```
    pub fn iter_scalars(&self) -> impl ExactSizeIterator<Item = S> + DoubleEndedIterator {
        let len = self.len();
        // Reconstituted once (a reference-count bump for an Arrow-backed column, one
        // rebuild for a decomposed one), then sliced per element below.
        let elements = self.values.as_ref().map(AnySerie::to_arrow);
        (0..len).map(move |index| {
            let element = elements
                .as_ref()
                .expect("a serie with elements has a column")
                .slice(index, 1);
            S::from_arrow(element.as_ref()).expect("a serie element reads back as its scalar")
        })
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> Default for TypedSerie<D, S> {
    /// The default serie scalar: the empty serie.
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<D: Clone, S> Clone for TypedSerie<D, S> {
    // Cloning bumps the child array's reference count — no element is copied.
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            values: self.values.clone(),
            element: PhantomData,
        }
    }
}

impl<D, S> PartialEq for TypedSerie<D, S> {
    // The backing arrays compare by value through `dyn Array` equality, so two
    // lists are equal when their elements (and nulls) are.
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl<D, S> Eq for TypedSerie<D, S> {}

impl<D: DataType + Default, S: Scalar<DataType = D>> From<Vec<S>> for TypedSerie<D, S> {
    /// A scalar holding the sequence `values`.
    fn from(values: Vec<S>) -> Self {
        Self::new(values)
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> From<Option<Vec<S>>> for TypedSerie<D, S> {
    /// A scalar holding the sequence, or the null scalar for `None`.
    fn from(values: Option<Vec<S>>) -> Self {
        match values {
            Some(values) => Self::new(values),
            None => Self::null(),
        }
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>> Scalar for TypedSerie<D, S> {
    type DataType = TypedSerieType<D>;
    type Value = AnySerie;

    fn data_type(&self) -> &TypedSerieType<D> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&AnySerie> {
        self.values.as_ref()
    }

    // A one-column table (a struct element renders one column per field), or `null`.
    fn display_with(&self, options: crate::DisplayOptions) -> String {
        match &self.values {
            None => "null".to_string(),
            Some(column) => crate::display::render_serie(column, "item", options),
        }
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The item serie is reconstituted into the one-element serie — a
        // reference-count bump, not a copy.
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
            values.to_arrow(),
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
        let data_type = TypedSerieType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a serie data type is a serie array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(AnySerie::from_arrow(array.value(0)))
        };
        Ok(Self {
            data_type,
            values,
            element: PhantomData,
        })
    }

    fn as_serie(&self) -> Result<crate::Serie, DataError> {
        Ok(self.erase())
    }
}

impl<D, S> crate::NestedSerie for TypedSerie<D, S> {
    fn child_serie_count(&self) -> usize {
        1
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        (index == 0).then(|| self.values.clone()).flatten()
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        (index == 0).then(|| "item".to_string())
    }
}

impl<D: DataType + Default, S: Scalar<DataType = D>>
    TypedScalar<TypedSerieType<D>, AnySerie, arrow_array::ListArray> for TypedSerie<D, S>
{
}

impl<T, D> ScalarFactory<Vec<T>> for TypedSerieType<D>
where
    D: ScalarFactory<T> + Default,
    D::Scalar: Scalar<DataType = D>,
{
    type Scalar = TypedSerie<D, D::Scalar>;

    /// A serie scalar holding the native `values`, each converted through the value
    /// type's own scalar factory.
    fn scalar(&self, values: Vec<T>) -> Self::Scalar {
        TypedSerie::new(
            values
                .into_iter()
                .map(|value| self.value_type().scalar(value))
                .collect(),
        )
    }

    /// The null serie scalar.
    fn null_scalar(&self) -> Self::Scalar {
        TypedSerie::null()
    }

    /// The default serie scalar: the empty serie.
    fn default_scalar(&self) -> Self::Scalar {
        TypedSerie::new(Vec::new())
    }
}
