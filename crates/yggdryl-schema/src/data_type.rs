//! The [`DataType`] base trait every yggdryl data type implements.

use crate::data_type_id::DataTypeId;

/// Behaviour shared by every yggdryl data type.
///
/// A type knows its canonical [`name`](DataType::name) and its
/// [`type_id`](DataType::type_id); the category predicates
/// ([`is_physical`](DataType::is_physical),
/// [`is_logical`](DataType::is_logical), [`is_nested`](DataType::is_nested))
/// follow from the id's block by default, so an implementor only supplies the two
/// required methods.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// struct Int32;
/// impl DataType for Int32 {
///     fn name(&self) -> &'static str { "int32" }
///     fn type_id(&self) -> DataTypeId { DataTypeId::Int32 }
/// }
///
/// assert_eq!(Int32.name(), "int32");
/// assert!(Int32.is_physical());
/// assert!(!Int32.is_nested());
/// ```
pub trait DataType {
    /// The canonical type name, e.g. `"int32"` or `"large_binary"`.
    fn name(&self) -> &'static str;

    /// The type's [`DataTypeId`] discriminant.
    fn type_id(&self) -> DataTypeId;

    /// Whether this is a physical (storage) type.
    fn is_physical(&self) -> bool {
        self.type_id().is_physical()
    }

    /// Whether this is a logical (reinterpreted) type.
    fn is_logical(&self) -> bool {
        self.type_id().is_logical()
    }

    /// Whether this is a nested (child-bearing) type.
    fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }
}
