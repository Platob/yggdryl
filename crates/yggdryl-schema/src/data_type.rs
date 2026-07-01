//! The [`DataType`] base trait.

use std::hash::Hasher;

use crate::data_type_id::DataTypeId;

/// The behaviour every data type shares: it knows its [`DataTypeId`] and its
/// [`type_name`](DataType::type_name). Each concrete type additionally carries a
/// category marker ([`PrimitiveType`](crate::PrimitiveType),
/// [`LogicalType`](crate::LogicalType) or [`NestedType`](crate::NestedType)).
///
/// Because a [`Field`](crate::Field) stores its type as a `Box<dyn DataType>`, the
/// trait is object-safe and carries the value-like hooks
/// ([`clone_box`](DataType::clone_box), [`dyn_eq`](DataType::dyn_eq),
/// [`dyn_hash`](DataType::dyn_hash)) that let the boxed type clone, compare and hash
/// — a concrete type writes them in a line or two.
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId};
///
/// let dt = BinaryType::new();
/// assert_eq!(dt.type_id(), DataTypeId::Binary);
/// assert_eq!(dt.type_name(), "binary");
/// ```
pub trait DataType: std::fmt::Debug {
    /// The discriminant identifying this type.
    fn type_id(&self) -> DataTypeId;

    /// The type's name (e.g. `"binary"`).
    fn type_name(&self) -> &str;

    /// Clones into a fresh box — the basis for cloning a `Box<dyn DataType>`.
    fn clone_box(&self) -> Box<dyn DataType>;

    /// Whether `other` is the same type with the same parameters.
    fn dyn_eq(&self, other: &dyn DataType) -> bool;

    /// Hashes the type's identity and parameters.
    fn dyn_hash(&self, state: &mut dyn Hasher);
}
