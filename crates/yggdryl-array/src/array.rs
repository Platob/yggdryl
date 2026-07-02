//! The abstract base every array implementation satisfies.

use core::fmt::Debug;

use arrow_buffer::NullBuffer;
use yggdryl_schema::DataType;

/// A typed column of values: the abstract base tying a [`DataType`], a length
/// and an optional validity bitmap together.
///
/// Implementors supply the three accessors; the null bookkeeping is provided
/// on top of them. Every yggdryl array — fixed-width, variable-size, nested —
/// implements this base over `arrow-buffer` buffers laid out per the Arrow
/// columnar spec.
///
/// ```
/// use yggdryl_array::{Array, PrimitiveArray};
/// use yggdryl_schema::Int64Type;
///
/// let column = PrimitiveArray::from_options(Int64Type, vec![Some(1), None]);
/// assert_eq!(column.len(), 2);
/// assert_eq!(column.null_count(), 1);
/// assert_eq!(column.is_null(1), Some(true));
/// assert_eq!(column.is_null(9), None); // out of bounds
/// ```
pub trait Array: Clone + Debug + Send + Sync + Sized + 'static {
    /// The data type of the array's elements.
    type DataType: DataType;

    /// The elements' data type.
    fn data_type(&self) -> &Self::DataType;

    /// The number of elements.
    fn len(&self) -> usize;

    /// The validity bitmap; `None` means every element is valid.
    fn validity(&self) -> Option<&NullBuffer>;

    /// Whether the array has no elements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of null elements.
    fn null_count(&self) -> usize {
        self.validity().map_or(0, NullBuffer::null_count)
    }

    /// Whether the element at `index` is null; `None` when out of bounds.
    fn is_null(&self, index: usize) -> Option<bool> {
        (index < self.len()).then(|| {
            self.validity()
                .is_some_and(|validity| validity.is_null(index))
        })
    }

    /// Whether the element at `index` is valid; `None` when out of bounds.
    fn is_valid(&self, index: usize) -> Option<bool> {
        self.is_null(index).map(|null| !null)
    }
}
