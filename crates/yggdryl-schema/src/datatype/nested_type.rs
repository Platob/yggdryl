//! The subtrait for data types containing child fields.

use crate::DataType;

/// A data type whose values contain child fields ([`ListType`](crate::ListType),
/// [`LargeListType`](crate::LargeListType), [`StructType`](crate::StructType),
/// [`MapType`](crate::MapType)).
///
/// Each nested type exposes its typed children as accessors on the concrete
/// type ([`ListType::child`](crate::ListType::child),
/// [`StructType::fields`](crate::StructType::fields)); heterogeneous children are
/// fields over the erased [`AnyDataType`](crate::AnyDataType).
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{Field, Int32Type, ListType, NestedType, TypedField};
///
/// let item = Arc::new(TypedField::from_parts("item", Int32Type, true, Default::default()));
/// assert_eq!(ListType::from_parts(item).num_children(), 1);
/// ```
pub trait NestedType: DataType {
    /// The number of child fields a value of this type contains.
    fn num_children(&self) -> usize;
}
