//! The subtrait for data types containing child fields.

use crate::DataType;

/// A data type whose values contain child fields ([`List`](crate::List),
/// [`LargeList`](crate::LargeList), [`Struct`](crate::Struct),
/// [`Map`](crate::Map)).
///
/// Each nested type exposes its typed children as accessors on the concrete
/// type ([`List::child`](crate::List::child),
/// [`Struct::fields`](crate::Struct::fields)); heterogeneous children are
/// fields over the erased [`AnyDataType`](crate::AnyDataType).
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{Field, Int32, List, NestedType, TypedField};
///
/// let item = Arc::new(TypedField::from_parts("item", Int32, true, Default::default()));
/// assert_eq!(List::from_parts(item).num_children(), 1);
/// ```
pub trait NestedType: DataType {
    /// The number of child fields a value of this type contains.
    fn num_children(&self) -> usize;
}
