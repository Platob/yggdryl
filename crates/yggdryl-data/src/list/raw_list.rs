//! The [`RawList`] base trait: the untyped surface of a list data type.

use crate::{Nested, RawDataType};

/// The untyped surface every list data type carries: a variable-length sequence of
/// one value type, exposing that value type.
///
/// It refines [`Nested`] (the single child is the item field) and is parameterised
/// by the value data type `D` so the concrete type is preserved for zero-cost
/// access, mirroring [`RawField`](crate::RawField) and
/// [`RawScalar`](crate::RawScalar). A value type with a codec also gets the typed
/// [`List`](crate::List) layer.
///
/// ```
/// use yggdryl_data::{Int64, ListType, Nested, RawDataType, RawList};
///
/// let list = ListType::new(Int64);
/// assert_eq!(list.value_type().name(), "int64");
/// assert_eq!(list.child_count(), 1);
/// ```
pub trait RawList<D: RawDataType>: Nested {
    /// The value type this list sequences.
    fn value_type(&self) -> &D;
}
