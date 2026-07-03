//! The typed [`List`] trait: a [`RawList`](super::RawList) whose value type has a
//! codec.

use super::RawList;
use crate::DataType;

/// A [`RawList`](super::RawList) whose value type is a typed [`DataType<T>`] — the
/// list's values have native Rust representation `Vec<T>`.
///
/// The concrete value type is the associated [`ValueType`](List::ValueType), so a
/// list has exactly one; `value_type` is inherited from
/// [`RawList`](super::RawList) and returns it. It also carries the
/// [`DataType<Vec<T>>`] surface itself: the codec concatenates and splits the
/// value type's per-element bytes, and the default is the empty list.
///
/// ```
/// use yggdryl_data::{DataType, Int64, List, ListType, RawScalar};
///
/// fn default_of<T, L: List<T>>(list: &L) -> Vec<T> {
///     list.default_value() // the empty list
/// }
///
/// let list = ListType::new(Int64);
/// assert_eq!(default_of(&list), Vec::<i64>::new());
/// assert!(!list.default_scalar().is_null()); // the empty list, not null
/// assert!(list.default_scalar().is_empty());
/// ```
pub trait List<T>: RawList<Self::ValueType> + DataType<Vec<T>> {
    /// The concrete value type of this list.
    type ValueType: DataType<T>;
}
