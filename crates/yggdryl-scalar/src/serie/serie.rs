//! The dynamic [`Serie`] scalar of the [`SerieType`](yggdryl_dtype::SerieType) data
//! type.

use crate::Scalar;
use arrow_array::ArrayRef;
// The `Serie` dtype trait is imported anonymously (its `item_field()` accessor is all
// we need) so it does not clash with this module's own `Serie` scalar type.
use yggdryl_dtype::Serie as _;
use yggdryl_dtype::{DataError, DataType, SerieType};

/// A single, possibly-null `list` value with its element type erased — *our array*
/// backed by one zero-copy Arrow child array, carrying a dynamic
/// [`SerieType`](yggdryl_dtype::SerieType).
///
/// It is the untyped base of the statically-typed
/// [`TypedSerie<D, S>`](crate::TypedSerie): it implements only the base [`Scalar`]
/// surface ([`to_arrow_scalar`](Scalar::to_arrow_scalar) /
/// [`to_arrow_array`](Scalar::to_arrow_array) / [`from_arrow`](Scalar::from_arrow),
/// all reference-count bumps) plus [`len`](Serie::len) / [`is_empty`](Serie::is_empty),
/// since the element scalar type is erased — the per-element scalar accessors and the
/// [`TypedScalar`](crate::TypedScalar) surface live on `TypedSerie<D, S>`, which
/// [`erase`](crate::TypedSerie::erase)s back to this type. (For `int64` there is the
/// concrete, buffer-backed [`Int64Serie`](crate::Int64Serie).)
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedSerie};
///
/// // A dynamic serie is reached by erasing a typed one, or from Arrow.
/// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]).erase();
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.len(), 2);
/// assert_eq!(numbers.data_type().name(), "list");
/// assert_eq!(yggdryl_scalar::Serie::from_arrow(numbers.to_arrow_scalar().as_ref()).unwrap(), numbers);
/// ```
#[derive(Debug, Clone)]
pub struct Serie {
    data_type: SerieType,
    values: Option<ArrayRef>,
}

impl Serie {
    /// A dynamic serie over an already-built Arrow `values` element array (shared
    /// zero-copy) of the given dynamic `data_type`, or the null serie for `None`.
    pub(crate) fn from_parts(data_type: SerieType, values: Option<ArrayRef>) -> Self {
        Self { data_type, values }
    }

    /// The number of elements, `0` when null or empty ([`is_null`](Scalar::is_null)
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
}

impl PartialEq for Serie {
    // The backing arrays compare by value through `dyn Array` equality, so two
    // series are equal when their elements (and nulls) are; null is distinct from
    // every present serie.
    fn eq(&self, other: &Self) -> bool {
        match (&self.values, &other.values) {
            (None, None) => true,
            (Some(left), Some(right)) => left.as_ref() == right.as_ref(),
            _ => false,
        }
    }
}

impl Eq for Serie {}

impl Scalar for Serie {
    type DataType = SerieType;
    type Value = dyn arrow_array::Array;

    fn data_type(&self) -> &SerieType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&(dyn arrow_array::Array + 'static)> {
        self.values.as_deref()
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
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

    fn to_arrow_array(&self) -> ArrayRef {
        // The element array itself (empty of the value type when null, told apart
        // from an empty serie by `is_null`).
        self.values.clone().unwrap_or_else(|| {
            arrow_array::new_empty_array(self.data_type.item_field().data_type())
        })
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout; the elements themselves are shared
        // zero-copy.
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
        Ok(Self { data_type, values })
    }
}
