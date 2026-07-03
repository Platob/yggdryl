//! The [`RawList`] base trait: the untyped surface of a list data type.

use crate::{RawDataType, RawNested};

/// The untyped surface every list data type carries: a variable-length sequence of
/// one value type, exposing that value type.
///
/// It refines [`RawNested`] (the single child is the item field) and is parameterised
/// by the value data type `D` so the concrete type is preserved for zero-cost
/// access, mirroring [`RawField`](crate::RawField) and
/// [`RawScalar`](crate::RawScalar). A value type with a codec also gets the typed
/// [`TypedList`](crate::TypedList) layer.
///
/// ```
/// use yggdryl_data::{Int64Type, ListType, RawDataType, RawList, RawNested};
///
/// let list = ListType::new(Int64Type);
/// assert_eq!(list.value_type().name(), "int64");
/// assert_eq!(list.child_count(), 1);
/// ```
pub trait RawList<D: RawDataType>: RawNested {
    /// The value type this list sequences.
    fn value_type(&self) -> &D;
}
