//! The typed [`TypedLogical`] trait: a [`Logical`](crate::Logical) whose values
//! have a native representation.

use super::{DataType, Logical, TypedDataType};

/// A [`Logical<S>`](crate::Logical) whose values have the native Rust representation
/// `T` — the typed layer over a storage-backed logical type.
///
/// The storage type `S` is an explicit generic parameter (a logical type has exactly
/// one), and `storage` is inherited from [`Logical`](crate::Logical). It also carries
/// the [`TypedDataType<T>`] surface itself, so a logical type reads and writes native
/// values while *storing* them as its physical storage. The generic
/// [`OptionalType<D>`](crate::OptionalType) is the crate's generic holder: a
/// `TypedLogical<UnionType, T>` over [`UnionType`](crate::UnionType) storage for any
/// value type `D: TypedDataType<T>`.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, OptionalType, TypedLogical, UnionType};
///
/// fn storage_name<S: DataType, L: TypedLogical<S, i64>>(logical: &L) -> String {
///     logical.storage().name().to_string()
/// }
///
/// let optional = OptionalType::new(Int64Type);
/// assert_eq!(storage_name::<UnionType, _>(&optional), "union");
/// ```
pub trait TypedLogical<S: DataType, T>: Logical<S> + TypedDataType<T> {}
