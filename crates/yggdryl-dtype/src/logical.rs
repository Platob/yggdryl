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
/// [`Optional<D>`](crate::Optional) is the crate's generic holder: a
/// `Logical<T>` over [`Union`](crate::Union) storage for any value type
/// `D: DataType<T>`.
///
/// ```
/// use yggdryl_dtype::{Int64, Logical, Optional, RawDataType, RawLogical};
///
/// fn storage_name<T, L: Logical<T>>(logical: &L) -> String {
///     logical.storage().name().to_string()
/// }
///
/// let optional = Optional::new(Int64);
/// assert_eq!(storage_name(&optional), "union");
/// ```
pub trait Logical<T>: RawLogical<Self::Storage> + DataType<T> {
    /// The physical storage type backing this logical type.
    type Storage: RawDataType;
}
