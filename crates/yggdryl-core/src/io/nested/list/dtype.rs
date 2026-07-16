//! [`ListType`] — the **list data-type descriptor**: the single element (item) field that defines a
//! list's shape, and the concrete implementor of the root [`DataType`](crate::io::DataType) for the
//! nested `list` family.

use crate::io::{AnyField, DataType, DataTypeId};

/// The **typed descriptor** of a list type — its single element (item) field (an [`AnyField`], leaf
/// or nested). A list's shape is exactly this one field, so `ListType` has no width of its own (it
/// reports `0`; a list is neither fixed-width nor variable-length). The named, nullable counterpart
/// is [`ListField`](super::ListField).
///
/// ```
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::nested::ListType;
/// use yggdryl_core::io::{AnyField, DataType};
///
/// let dt = ListType::new(AnyField::leaf(Field::new("item", &PrimitiveType::<i32>::new(), true)));
/// assert_eq!(dt.name(), "list");
/// assert!(dt.is_list());
/// assert_eq!(dt.item().name(), "item");
/// ```
// DESIGN: only Arrow `List` (i32 offsets) is modeled here. `LargeList` (i64 offsets) and
// `FixedSizeList` (a fixed element count per row) are reserved at `DataTypeId` 0x0211..=0x021F and
// will get their own descriptors when added — a list's offset width is a type-id property, not a
// field of `ListType`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListType {
    item: Box<AnyField>,
}

impl ListType {
    /// A list type from its single element (item) field.
    pub fn new(item: AnyField) -> Self {
        Self {
            item: Box::new(item),
        }
    }

    /// The element (item) field.
    pub fn item(&self) -> &AnyField {
        &self.item
    }
}

impl DataType for ListType {
    fn name(&self) -> &'static str {
        "list"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::List
    }

    /// The Arrow `List(item field)` type (feature `arrow`) — **recursive**, the item mapped by its
    /// [`AnyField::to_arrow`]. Overrides the id-level shell default (which cannot supply the item).
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::List(std::sync::Arc::new(self.item.to_arrow()))
    }
}
