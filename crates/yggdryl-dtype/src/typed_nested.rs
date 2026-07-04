//! The typed [`TypedNested`] trait: a [`Nested`](crate::Nested) whose values have
//! a native representation.

use super::{Nested, TypedDataType};

/// A [`Nested`](crate::Nested) whose values have the native Rust representation `T` —
/// the typed layer over a child-bearing type.
///
/// It carries the [`TypedDataType<T>`] surface itself, so a nested type reads and
/// writes native values (a sequence, a row) while its children stay Arrow fields.
/// The generic [`TypedSerieType<D>`](crate::TypedSerieType) and
/// [`TypedMapType<K, V>`](crate::TypedMapType) are the crate's generic holders: a
/// `TypedNested<Vec<T>>` and a `TypedNested<Vec<(TK, TV)>>` for any child types with
/// codecs (their dynamic bases [`SerieType`](crate::SerieType) /
/// [`MapType`](crate::MapType), like [`StructType`](crate::StructType) and
/// [`UnionType`](crate::UnionType), stay untyped).
///
/// ```
/// use yggdryl_dtype::{Int64Type, TypedMapType, TypedSerieType, TypedDataType, TypedNested, UInt8Type};
///
/// fn children_of<T, N: TypedNested<T>>(nested: &N) -> usize {
///     nested.child_count()
/// }
///
/// assert_eq!(children_of::<Vec<i64>, _>(&TypedSerieType::new(Int64Type)), 1);
/// assert_eq!(children_of::<Vec<(u8, i64)>, _>(&TypedMapType::new(UInt8Type, Int64Type)), 1);
/// ```
pub trait TypedNested<T>: Nested + TypedDataType<T> {}
