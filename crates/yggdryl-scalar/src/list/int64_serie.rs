//! The [`Int64Serie`] scalar: a list of `int64` borrowing raw Arrow buffers.

use crate::{Int64, RawScalar, Scalar};
use arrow_buffer::{NullBuffer, ScalarBuffer};
use yggdryl_dtype::{DataError, List, RawDataType};

/// A single, possibly-null list of `int64` — *our array*, borrowing the raw Arrow
/// buffers ([`ScalarBuffer<i64>`] elements plus an optional [`NullBuffer`])
/// zero-copy, optimized for native `i64` access.
///
/// Where the generic [`Serie`](crate::Serie) holds an opaque Arrow array
/// handle and goes through the element scalars' Arrow round trip, `Int64Serie`
/// holds the underlying buffers themselves: [`values`](Int64Serie::values) borrows
/// the whole element buffer as `&[i64]` without copying,
/// [`get_at`](Int64Serie::get_at) reads one element null-aware as any native
/// Rust target, and the *scalar accessor*
/// [`get_scalar_at`](Int64Serie::get_scalar_at) hands back an [`Int64`] (the
/// inner null scalar for a null slot). [`from_io`](Int64Serie::from_io) /
/// [`pwrite_io`](Int64Serie::pwrite_io) bridge the elements to any
/// `yggdryl-core` positioned-IO resource through the little-endian primitive
/// helpers. The optimized
/// [`to_arrow`](RawScalar::to_arrow) / [`from_arrow`](RawScalar::from_arrow)
/// reassemble and take apart the Arrow form around the same shared buffers —
/// reference-count bumps, never element copies — so the type moves across the
/// Arrow FFI boundary without copying elements.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::RawDataType;
/// use yggdryl_scalar::{Int64, Int64Serie, RawScalar};
///
/// let numbers = Int64Serie::from(vec![1, 2, 3]);
/// assert_eq!(numbers.len(), 3);
/// assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // zero-copy buffer borrow
/// assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2);
/// assert_eq!(numbers.get_at::<i32>(1).unwrap(), 2); // converted, exact-or-error
/// assert_eq!(numbers.get_scalar_at(1), Some(Int64::new(2)));
/// assert_eq!(numbers.data_type().name(), "list");
///
/// // Nulls are per element, read null-aware.
/// let sparse = Int64Serie::from(vec![Some(1), None]);
/// assert!(sparse.get_at::<i64>(1).is_err()); // a null element holds no value
/// assert_eq!(sparse.get_scalar_at(1), Some(Int64::null()));
/// assert_eq!(sparse.values().map(<[i64]>::len), Some(2)); // raw slots, nulls included
///
/// // The Arrow round trip shares the buffers — no element is copied.
/// let arrow = numbers.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Int64Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
///
/// assert!(Int64Serie::null().is_null());
/// ```
#[derive(Debug, Clone)]
pub struct Int64Serie {
    data_type: List<yggdryl_dtype::Int64>,
    values: Option<ScalarBuffer<i64>>,
    nulls: Option<NullBuffer>,
}

impl Int64Serie {
    /// An array borrowing the element buffer `values` and the per-element `nulls`
    /// zero-copy. A null buffer whose length differs from the element buffer's
    /// errors with [`DataError::MismatchedNullBufferLength`].
    pub fn new(values: ScalarBuffer<i64>, nulls: Option<NullBuffer>) -> Result<Self, DataError> {
        if let Some(nulls) = &nulls {
            if nulls.len() != values.len() {
                return Err(DataError::MismatchedNullBufferLength {
                    expected: values.len(),
                    got: nulls.len(),
                });
            }
        }
        Ok(Self::from_parts(values, nulls))
    }

    /// The null list scalar.
    pub fn null() -> Self {
        Self {
            data_type: List::new(yggdryl_dtype::Int64),
            values: None,
            nulls: None,
        }
    }

    // The unchecked constructor; callers guarantee `nulls` matches `values` in
    // length. An all-valid null buffer is dropped so the stored form is canonical
    // and the `nulls()` contract (`None` when every element is valid) holds on
    // every construction path.
    fn from_parts(values: ScalarBuffer<i64>, nulls: Option<NullBuffer>) -> Self {
        Self {
            data_type: List::new(yggdryl_dtype::Int64),
            values: Some(values),
            nulls: nulls.filter(|nulls| nulls.null_count() > 0),
        }
    }

    /// The number of elements, `0` when null or empty ([`is_null`](RawScalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.values.as_ref().map_or(0, ScalarBuffer::len)
    }

    /// Whether the sequence holds no elements (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The whole element buffer as a native slice, borrowed without copying —
    /// including the (arbitrary) slots under null elements; pair with
    /// [`get_at`](Int64Serie::get_at) or
    /// [`get_scalar_at`](Int64Serie::get_scalar_at) for null-aware reads.
    pub fn values(&self) -> Option<&[i64]> {
        self.values.as_deref()
    }

    /// The per-element null buffer, when any element is null — `None` both for an
    /// all-valid array (an all-valid buffer is dropped at construction, so the
    /// stored form is canonical) and for the null list.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// The elements as an Arrow [`arrow_array::Int64Array`], reassembled around the
    /// same shared buffers (a reference-count bump, not a copy), or `None` when the
    /// list is null.
    pub fn array(&self) -> Option<arrow_array::Int64Array> {
        self.values
            .as_ref()
            .map(|values| arrow_array::Int64Array::new(values.clone(), self.nulls.clone()))
    }

    /// The element at `index` read as the native Rust type `T` — the generic
    /// native accessor, answered straight from the borrowed buffers (no Arrow
    /// slicing): the element becomes its [`Int64`] scalar and `T` reads through
    /// the `as_*` contract via [`FromScalar`](crate::FromScalar).
    ///
    /// A null serie errors with [`DataError::NullValue`], an index past the end
    /// with [`DataError::OutOfBounds`], and a null or non-representable element
    /// with the `as_*` contract's own errors.
    pub fn get_at<T: crate::FromScalar>(&self, index: usize) -> Result<T, DataError> {
        let values = self.values.as_ref().ok_or(DataError::NullValue)?;
        if index >= values.len() {
            return Err(DataError::OutOfBounds {
                index,
                len: values.len(),
            });
        }
        let scalar = if self
            .nulls
            .as_ref()
            .is_none_or(|nulls| nulls.is_valid(index))
        {
            Int64::new(values[index])
        } else {
            Int64::null()
        };
        T::from_scalar(&scalar)
    }

    /// A serie read out of a `yggdryl-core` positioned-IO resource: the whole
    /// byte size split into little-endian `i64` elements via
    /// [`pread_i64`](yggdryl_core::RawIOBase::pread_i64), all valid (the byte
    /// layer carries no nulls). A byte size that is not a whole number of
    /// elements errors with [`DataError::InvalidByteLength`].
    pub fn from_io(io: &(impl yggdryl_core::RawIOBase + ?Sized)) -> Result<Self, DataError> {
        let size = io.byte_size();
        if !size.is_multiple_of(8) {
            return Err(DataError::InvalidByteLength {
                expected: size.div_ceil(8) * 8,
                got: size,
            });
        }
        let values = (0..size / 8)
            .map(|index| io.pread_i64(index * 8, yggdryl_core::Whence::Start))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from(values))
    }

    /// Write every element buffer slot into a `yggdryl-core` positioned-IO
    /// resource via [`pwrite_i64`](yggdryl_core::RawIOBase::pwrite_i64), element
    /// `index` landing at `position + index * 8` relative to `whence` — the raw
    /// slots under null elements included, like [`values`](Int64Serie::values).
    /// A null serie errors with [`DataError::NullValue`].
    pub fn pwrite_io(
        &self,
        io: &mut (impl yggdryl_core::RawIOBase + ?Sized),
        position: usize,
        whence: yggdryl_core::Whence,
    ) -> Result<(), DataError> {
        let values = self.values.as_ref().ok_or(DataError::NullValue)?;
        for (index, value) in values.iter().enumerate() {
            io.pwrite_i64(position + index * 8, whence, *value)?;
        }
        Ok(())
    }

    /// The element at `index` as a scalar (a null element is the null scalar), or
    /// `None` when the list is null or `index` is out of bounds.
    pub fn get_scalar_at(&self, index: usize) -> Option<Int64> {
        let values = self.values.as_ref()?;
        if index >= values.len() {
            return None;
        }
        Some(
            if self
                .nulls
                .as_ref()
                .is_none_or(|nulls| nulls.is_valid(index))
            {
                Int64::new(values[index])
            } else {
                Int64::null()
            },
        )
    }
}

impl Default for Int64Serie {
    /// The default list scalar: the empty list.
    fn default() -> Self {
        Self::from_parts(ScalarBuffer::from(Vec::new()), None)
    }
}

impl PartialEq for Int64Serie {
    // Compared logically, like Arrow arrays: element values and per-element
    // nullness — an all-valid null buffer equals no null buffer at all.
    fn eq(&self, other: &Self) -> bool {
        match (self.array(), other.array()) {
            (None, None) => true,
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }
}

impl Eq for Int64Serie {}

impl From<ScalarBuffer<i64>> for Int64Serie {
    /// An all-valid array borrowing the element buffer zero-copy.
    fn from(values: ScalarBuffer<i64>) -> Self {
        Self::from_parts(values, None)
    }
}

impl From<arrow_array::Int64Array> for Int64Serie {
    /// An array taking over the Arrow array's buffers, shared zero-copy.
    fn from(values: arrow_array::Int64Array) -> Self {
        let (_, values, nulls) = values.into_parts();
        Self::from_parts(values, nulls)
    }
}

impl From<Vec<i64>> for Int64Serie {
    /// An array over the native values, moved into the element buffer.
    fn from(values: Vec<i64>) -> Self {
        Self::from_parts(ScalarBuffer::from(values), None)
    }
}

impl From<Vec<Option<i64>>> for Int64Serie {
    /// An array over the native values with per-element nulls.
    fn from(values: Vec<Option<i64>>) -> Self {
        Self::from(arrow_array::Int64Array::from(values))
    }
}

impl RawScalar<List<yggdryl_dtype::Int64>> for Int64Serie {
    /// The raw element buffer — like [`values`](Int64Serie::values), it includes
    /// the slots under null elements.
    type Value = [i64];

    fn data_type(&self) -> &List<yggdryl_dtype::Int64> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&[i64]> {
        self.values.as_deref()
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&RawDataType::to_arrow(&self.data_type), 1);
        };
        // The buffers are reassembled into the one-element list — reference-count
        // bumps, not copies.
        let elements = arrow_array::Int64Array::new(values.clone(), self.nulls.clone());
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
            std::sync::Arc::new(elements),
            None,
        )
        .expect("a one-element list of int64 elements is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // Validates the list-of-int64 layout, then takes the buffers apart and
        // shares them.
        List::<yggdryl_dtype::Int64>::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a list data type is a list array");
        if arrow_array::Array::is_null(array, 0) {
            return Ok(Self::null());
        }
        let elements = array.value(0);
        let elements = elements
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .expect("a validated list of int64 has int64 elements");
        Ok(Self::from_parts(
            elements.values().clone(),
            arrow_array::Array::nulls(elements).cloned(),
        ))
    }
}

impl Scalar<[i64]> for Int64Serie {
    type Type = List<yggdryl_dtype::Int64>;
}
