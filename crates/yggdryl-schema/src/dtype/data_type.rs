//! The [`DataType`] base trait.

use std::hash::{Hash, Hasher};

use crate::dtype::DataTypeId;
use crate::nested_fields::NestedFields;

/// The behaviour every data type shares: it knows its [`DataTypeId`] and its
/// [`type_name`](DataType::type_name), and — as a [`NestedFields`] — its child
/// fields. Each concrete type additionally carries a category marker
/// ([`PrimitiveType`](crate::PrimitiveType), [`LogicalType`](crate::LogicalType) or
/// [`NestedType`](crate::NestedType)).
///
/// Because a nested type stores its type as a `Box<dyn DataType>`, the trait is
/// object-safe and carries the value-like hooks ([`clone_box`](DataType::clone_box),
/// [`dyn_eq`](DataType::dyn_eq), [`dyn_hash`](DataType::dyn_hash)); the two `dyn_*`
/// default to comparing / hashing the [`type_id`](DataType::type_id), which a
/// parametrized type overrides.
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId, NestedFields};
///
/// let dt = BinaryType::new();
/// assert_eq!(dt.type_id(), DataTypeId::Binary);
/// assert_eq!(dt.type_name(), "binary");
/// assert!(dt.children_fields().is_empty()); // a primitive has no children
/// ```
pub trait DataType: NestedFields + std::fmt::Debug {
    /// The discriminant identifying this type.
    fn type_id(&self) -> DataTypeId;

    /// The type's name (e.g. `"binary"`).
    fn type_name(&self) -> &str;

    /// Clones into a fresh box — the basis for cloning a `Box<dyn DataType>`.
    fn clone_box(&self) -> Box<dyn DataType>;

    /// Whether `other` is the same type with the same parameters. The default
    /// compares [`type_id`](DataType::type_id) — enough for a parameterless type; a
    /// parametrized type overrides it.
    fn dyn_eq(&self, other: &dyn DataType) -> bool {
        self.type_id() == other.type_id()
    }

    /// Hashes the type's identity and parameters. The default hashes
    /// [`type_id`](DataType::type_id); a parametrized type overrides it.
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.type_id().hash(&mut state);
    }
}
