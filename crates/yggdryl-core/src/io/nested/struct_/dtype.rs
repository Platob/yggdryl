//! [`StructType`] — the **struct data-type descriptor**: the ordered set of child fields that
//! defines a struct's shape, and the concrete implementor of the root
//! [`DataType`](crate::io::DataType) for the nested `struct` family.

use crate::io::nested::ColumnField;
use crate::io::{DataType, DataTypeId};

/// The **typed descriptor** of a struct type — its ordered, named child fields. A struct's shape is
/// exactly this list of [`ColumnField`]s (each itself a leaf or a nested field), so `StructType`
/// carries no width of its own (it reports `0`; a struct is neither fixed-width nor
/// variable-length). The named, nullable counterpart is [`StructField`](super::StructField).
///
/// ```
/// use yggdryl_core::io::DataType;
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
/// use yggdryl_core::io::nested::{ColumnField, StructType};
///
/// let dt = StructType::new(vec![
///     ColumnField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false)),
///     ColumnField::leaf(Field::new("name", &PrimitiveType::<i32>::new(), true)),
/// ]);
/// assert_eq!(dt.name(), "struct");
/// assert_eq!(dt.num_fields(), 2);
/// assert!(dt.is_struct());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct StructType {
    children: Vec<ColumnField>,
}

impl StructType {
    /// A struct type from its ordered child fields.
    pub fn new(children: Vec<ColumnField>) -> Self {
        Self { children }
    }

    /// The child fields, in order.
    pub fn fields(&self) -> &[ColumnField] {
        &self.children
    }

    /// The number of child fields.
    pub fn num_fields(&self) -> usize {
        self.children.len()
    }

    /// The child field at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&ColumnField> {
        self.children.get(index)
    }
}

impl DataType for StructType {
    fn name(&self) -> &'static str {
        "struct"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    /// The Arrow `Struct(child fields)` type (feature `arrow`) — **recursive**, each child mapped by
    /// its [`ColumnField::to_arrow`]. Overrides the id-level shell default (which cannot supply children).
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        let fields: Vec<arrow_schema::Field> =
            self.children.iter().map(ColumnField::to_arrow).collect();
        arrow_schema::DataType::Struct(arrow_schema::Fields::from(fields))
    }
}
