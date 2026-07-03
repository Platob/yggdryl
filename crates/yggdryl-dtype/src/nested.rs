//! The typed [`Nested`] trait: a [`RawNested`](crate::RawNested) whose values have
//! a native representation.

use super::{DataType, RawNested};

/// A [`RawNested`](crate::RawNested) whose values have the native Rust
/// representation `T` — the typed layer over a child-bearing type.
///
/// It carries the [`DataType<T>`] surface itself, so a nested type reads and
/// writes native values (a sequence, a row) while its children stay Arrow fields.
/// The generic [`List<D>`](crate::List) and [`Map<K, V>`](crate::Map) are the
/// crate's generic holders: a `Nested<Vec<T>>` and a `Nested<Vec<(TK, TV)>>` for
/// any child types with codecs (the dynamic [`Struct`](crate::Struct) and
/// [`Union`](crate::Union), whose children are only known at runtime, stay
/// raw-only).
///
/// ```
/// use yggdryl_dtype::{DataType, Int64, List, Map, Nested, RawNested, UInt8};
///
/// fn children_of<T, N: Nested<T>>(nested: &N) -> usize {
///     nested.child_count()
/// }
///
/// assert_eq!(children_of::<Vec<i64>, _>(&List::new(Int64)), 1);
/// assert_eq!(children_of::<Vec<(u8, i64)>, _>(&Map::new(UInt8, Int64)), 1);
/// ```
pub trait Nested<T>: RawNested + DataType<T> {}
