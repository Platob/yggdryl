//! The typed [`TypedNested`] trait: a [`Nested`](crate::Nested) whose values have
//! a native representation.

use super::{Nested, TypedDataType};

/// A [`Nested`](crate::Nested) whose values have the native Rust representation `T` —
/// the typed layer over a child-bearing type.
///
/// It carries the [`TypedDataType<T>`] surface itself, so a nested type reads and
/// writes native values (a sequence, a row) while its children stay Arrow fields.
/// The generic [`ListType<D>`](crate::ListType) and [`MapType<K, V>`](crate::MapType)
/// are the crate's generic holders: a `TypedNested<Vec<T>>` and a
/// `TypedNested<Vec<(TK, TV)>>` for any child types with codecs (the dynamic
/// [`StructType`](crate::StructType) and [`UnionType`](crate::UnionType), whose
/// children are only known at runtime, stay untyped).
///
/// ```
/// use yggdryl_dtype::{Int64Type, ListType, MapType, TypedDataType, TypedNested, UInt8Type};
///
/// fn children_of<T, N: TypedNested<T>>(nested: &N) -> usize {
///     nested.child_count()
/// }
///
/// assert_eq!(children_of::<Vec<i64>, _>(&ListType::new(Int64Type)), 1);
/// assert_eq!(children_of::<Vec<(u8, i64)>, _>(&MapType::new(UInt8Type, Int64Type)), 1);
/// ```
pub trait TypedNested<T>: Nested + TypedDataType<T> {}
