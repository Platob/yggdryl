//! The typed [`Logical`] trait: a [`RawLogical`](crate::RawLogical) whose values
//! have a native representation.

use super::{DataType, RawDataType, RawLogical};

/// A [`RawLogical`](crate::RawLogical) whose values have the native Rust
/// representation `T` — the typed layer over a storage-backed logical type.
///
/// The concrete storage type is the associated [`Storage`](Logical::Storage), so a
/// logical type has exactly one; `storage` is inherited from
/// [`RawLogical`](crate::RawLogical) and returns it. It also carries the
/// [`DataType<T>`] surface itself, so a logical type reads and writes native
/// values while *storing* them as its physical storage. The generic
/// [`OptionalType<D>`](crate::OptionalType) is the crate's generic holder: a
/// `Logical<T>` over [`UnionType`](crate::UnionType) storage for any value type
/// `D: DataType<T>`.
///
/// ```
/// use yggdryl_data::{Int64Type, Logical, OptionalType, RawDataType, RawLogical};
///
/// fn storage_name<T, L: Logical<T>>(logical: &L) -> String {
///     logical.storage().name().to_string()
/// }
///
/// let optional = OptionalType::new(Int64Type);
/// assert_eq!(storage_name(&optional), "union");
/// ```
pub trait Logical<T>: RawLogical<Self::Storage> + DataType<T> {
    /// The physical storage type backing this logical type.
    type Storage: RawDataType;
}
