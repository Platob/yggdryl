//! The typed [`TypedLogical`] trait: a [`Logical`](crate::Logical) whose values
//! have a native representation.

use super::{Logical, TypedDataType};

/// A [`Logical`](crate::Logical) whose values have the native Rust representation
/// `T` — the typed layer over a storage-backed logical type.
///
/// `storage` (and the associated [`Storage`](crate::Logical::Storage)) is inherited
/// from [`Logical`](crate::Logical). It also carries the [`TypedDataType<T>`] surface
/// itself, so a logical type reads and writes native values while *storing* them as
/// its physical storage. The generic
/// [`TypedOptionalType<D>`](crate::TypedOptionalType) is the crate's generic holder:
/// a `TypedLogical<T>` over [`UnionType`](crate::UnionType) storage for any value
/// type `D: TypedDataType<T>` (its dynamic base [`OptionalType`](crate::OptionalType)
/// stays untyped).
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, TypedLogical, TypedOptionalType};
///
/// fn storage_name<L: TypedLogical<i64>>(logical: &L) -> String {
///     logical.storage().name().to_string()
/// }
///
/// let optional = TypedOptionalType::new(Int64Type);
/// assert_eq!(storage_name(&optional), "union");
/// ```
pub trait TypedLogical<T>: Logical + TypedDataType<T> {}
