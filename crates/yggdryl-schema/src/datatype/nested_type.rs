//! The subtrait for data types containing child fields.

use crate::DataType;

/// A data type whose values contain child fields ([`List`](crate::List),
/// [`LargeList`](crate::LargeList), and — as they land — struct and map
/// types).
///
/// Homogeneous nested types expose their typed children as accessors on the
/// concrete type ([`List::child`](crate::List::child)); erased child access
/// arrives with the dynamic layer above this crate.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{Field, Int32, List, NestedType};
///
/// let item = Arc::new(Field::from_parts("item", Int32, true, Default::default()));
/// assert_eq!(List::from_parts(item).num_children(), 1);
/// ```
pub trait NestedType: DataType {
    /// The number of child fields a value of this type contains.
    fn num_children(&self) -> usize;
}
