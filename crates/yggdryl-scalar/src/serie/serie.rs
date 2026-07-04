//! The dynamic [`Serie`] scalar of the [`SerieType`](yggdryl_dtype::SerieType) data
//! type.

use crate::{AnySerie, Scalar};
use arrow_array::ArrayRef;
// The `Serie` dtype trait is imported anonymously (its `item_field()` accessor is all
// we need) so it does not clash with this module's own `Serie` scalar type.
use yggdryl_dtype::Serie as _;
use yggdryl_dtype::{DataError, DataType, SerieType};

/// A single, possibly-null `list` value with its element type erased — *our array*,
/// holding its items as the crate's own [`AnySerie`] column, carrying a dynamic
/// [`SerieType`](yggdryl_dtype::SerieType).
///
/// The items live in an [`AnySerie`] — integer elements decomposed to their raw
/// buffers, anything else zero-copy Arrow — so the Arrow forms are reconstituted on
/// demand ([`to_arrow_scalar`](Scalar::to_arrow_scalar) /
/// [`to_arrow_array`](Scalar::to_arrow_array), reference-count bumps) and
/// [`from_arrow`](Scalar::from_arrow) *decomposes* the incoming array. It is the
/// untyped base of the statically-typed [`TypedSerie<D, S>`](crate::TypedSerie): it
/// implements only the base [`Scalar`] surface plus [`len`](Serie::len) /
/// [`is_empty`](Serie::is_empty) and the [`NestedSerie`](crate::NestedSerie) child
/// access, since the element scalar type is erased — the per-element scalar
/// accessors and the [`TypedScalar`](crate::TypedScalar) surface live on
/// `TypedSerie<D, S>`, which [`erase`](crate::TypedSerie::erase)s back to this type.
/// (For `int64` there is the concrete, buffer-backed
/// [`Int64Serie`](crate::Int64Serie).)
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_scalar::{Int64Scalar, NestedSerie, Scalar, TypedSerie};
///
/// // A dynamic serie is reached by erasing a typed one, or from Arrow.
/// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]).erase();
/// assert!(!numbers.is_null());
/// assert_eq!(numbers.len(), 2);
/// assert_eq!(numbers.data_type().name(), "list");
/// assert_eq!(numbers.child_serie_at(0).unwrap().len(), 2); // the item serie
/// assert_eq!(yggdryl_scalar::Serie::from_arrow(numbers.to_arrow_scalar().as_ref()).unwrap(), numbers);
/// ```
#[derive(Debug, Clone)]
pub struct Serie {
    data_type: SerieType,
    values: Option<AnySerie>,
}

impl Serie {
    /// A dynamic serie over an already-built item serie `values` (shared zero-copy)
    /// of the given dynamic `data_type`, or the null serie for `None`.
    pub(crate) fn from_parts(data_type: SerieType, values: Option<AnySerie>) -> Self {
        Self { data_type, values }
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

    /// An iterator over the elements as type-erased [`AnyScalar`](crate::AnyScalar)
    /// atoms, in order (a null element is the atom's null; a null serie yields
    /// nothing). Each step reads one element straight from the decomposed column —
    /// a buffer read for a numeric element, a one-element slice for anything else —
    /// so the walk never reconstitutes the whole Arrow array. The iterator owns a
    /// reference-counted view of the column, borrowing nothing, and is
    /// [`ExactSizeIterator`] / [`DoubleEndedIterator`].
    ///
    /// ```
    /// use yggdryl_scalar::{Int64Scalar, Scalar, TypedSerie};
    ///
    /// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]).erase();
    /// let values: Vec<i64> = numbers
    ///     .iter_scalars()
    ///     .map(|atom| atom.int64().unwrap().as_i64().unwrap()) // the zero-copy typed view
    ///     .collect();
    /// assert_eq!(values, vec![1, 2]);
    /// assert_eq!(numbers.iter_scalars().len(), 2); // exact size, no walk
    /// ```
    pub fn iter_scalars(
        &self,
    ) -> impl ExactSizeIterator<Item = crate::AnyScalar> + DoubleEndedIterator {
        let len = self.len();
        // A reference-count bump of the decomposed column; `get_scalar` then reads
        // each element from its buffers (no whole-array reconstitution).
        let values = self.values.clone();
        (0..len).map(move |index| {
            values
                .as_ref()
                .expect("a serie with elements has a column")
                .get_scalar(index)
                .expect("index within bounds")
        })
    }
}

impl PartialEq for Serie {
    // Compared logically, like Arrow arrays: two series are equal when their item
    // series are; null is distinct from every present serie.
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl Eq for Serie {}

impl crate::NestedSerie for Serie {
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

impl Scalar for Serie {
    type DataType = SerieType;
    type Value = AnySerie;

    fn data_type(&self) -> &SerieType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&AnySerie> {
        self.values.as_ref()
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The item serie is reconstituted into the one-element serie —
        // reference-count bumps, not copies.
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
            values.to_arrow(),
            None,
        )
        .expect("a one-element serie of the value type's child is valid");
        std::sync::Arc::new(array)
    }

    fn to_arrow_array(&self) -> ArrayRef {
        // The item serie itself, reconstituted (empty of the value type when null,
        // told apart from an empty serie by `is_null`).
        self.values.as_ref().map_or_else(
            || arrow_array::new_empty_array(self.data_type.item_field().data_type()),
            AnySerie::to_arrow,
        )
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout; the items are decomposed into the
        // crate's own serie, sharing the buffers zero-copy.
        let data_type = SerieType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a serie data type is a serie array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(AnySerie::from_arrow(array.value(0)))
        };
        Ok(Self { data_type, values })
    }

    fn as_serie(&self) -> Result<Serie, DataError> {
        Ok(self.clone())
    }
}
