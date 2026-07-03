//! The typed [`TypedList`] trait: a [`List`](super::List) whose value type has a
//! codec.

use super::List;
use crate::TypedDataType;

/// A [`List`](super::List) whose value type is a typed [`TypedDataType<T>`] — the
/// list's values have native Rust representation `Vec<T>`.
///
/// The concrete value type is the associated [`ValueType`](TypedList::ValueType), so a
/// list has exactly one; `value_type` is inherited from [`List`](super::List) and
/// returns it. It also carries the [`TypedDataType<Vec<T>>`] surface itself: the
/// codec concatenates and splits the value type's per-element bytes, and the default
/// is the empty list.
///
/// ```
/// use yggdryl_dtype::{Int64Type, ListType, TypedDataType, TypedList};
///
/// fn default_of<T, L: TypedList<T>>(list: &L) -> Vec<T> {
///     list.default_value() // the empty list
/// }
///
/// let list = ListType::new(Int64Type);
/// assert_eq!(default_of(&list), Vec::<i64>::new());
/// ```
pub trait TypedList<T>: List<Self::ValueType> + TypedDataType<Vec<T>> {
    /// The concrete value type of this list.
    type ValueType: TypedDataType<T>;
}
