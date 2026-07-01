//! The [`Field`] base trait.

use std::hash::{Hash, Hasher};

use crate::dtype::DataType;
use crate::field::Metadata;
use crate::nested_fields::NestedFields;

/// A named column: its `name`, its [`DataType`], and optional byte-keyed
/// [`Metadata`]. Mirrors [`DataType`] — it is object-safe, is a [`NestedFields`], and
/// carries the same value-like hooks ([`clone_box`](Field::clone_box),
/// [`dyn_eq`](Field::dyn_eq), [`dyn_hash`](Field::dyn_hash)); the two `dyn_*` default
/// to comparing / hashing the name, data type and metadata. Each concrete field also
/// carries a category marker ([`PrimitiveField`](crate::PrimitiveField),
/// [`LogicalField`](crate::LogicalField) or [`NestedField`](crate::NestedField)).
///
/// ```
/// use yggdryl_schema::{DataTypeId, Field, Int32Field, NestedFields};
///
/// let field = Int32Field::new("count");
/// assert_eq!(field.name(), "count");
/// assert_eq!(field.dtype().type_id(), DataTypeId::Int32);
/// assert!(field.children_fields().is_empty());
/// ```
pub trait Field: NestedFields + std::fmt::Debug {
    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn dtype(&self) -> &dyn DataType;

    /// The field's metadata, if any.
    fn metadata(&self) -> Option<&Metadata>;

    /// Clones into a fresh box — the basis for cloning a `Box<dyn Field>`.
    fn clone_box(&self) -> Box<dyn Field>;

    /// Whether `other` has the same name, data type and metadata.
    fn dyn_eq(&self, other: &dyn Field) -> bool {
        self.name() == other.name()
            && self.dtype().dyn_eq(other.dtype())
            && self.metadata() == other.metadata()
    }

    /// Hashes the name, data type and metadata.
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.name().hash(&mut state);
        self.dtype().dyn_hash(&mut state);
        self.metadata().hash(&mut state);
    }
}
