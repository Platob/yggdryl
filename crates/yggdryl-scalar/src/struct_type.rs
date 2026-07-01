//! The [`StructType`] nested data type.

use yggdryl_schema::{DataType, DataTypeId};

use crate::{AnyField, Struct};

/// A struct type — a composite of named, heterogeneous child [`AnyField`]s. It is a
/// [`DataType`] over the [`Struct`] value (an array of `Any`). Because a child
/// field can itself be a struct, nesting is fully recursive; an Arrow *schema* is just
/// a [`StructField`](crate::StructField) wrapping one of these.
///
/// ```
/// use yggdryl_scalar::{AnyField, StructType};
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let ty = StructType::new(vec![AnyField::int64("id"), AnyField::utf8("tag")]);
/// assert_eq!(ty.type_id(), DataTypeId::Struct);
/// assert_eq!(ty.len(), 2);
/// assert_eq!(ty.field_by("tag").map(AnyField::name), Some("tag"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructType {
    fields: Vec<AnyField>,
}

impl StructType {
    /// A struct type from its child fields, in order.
    pub fn new(fields: Vec<AnyField>) -> Self {
        Self { fields }
    }

    /// The child fields, in order.
    pub fn fields(&self) -> &[AnyField] {
        &self.fields
    }

    /// The child field at `index`, if any.
    pub fn field_at(&self, index: usize) -> Option<&AnyField> {
        self.fields.get(index)
    }

    /// The first child field named `name`, if any.
    pub fn field_by(&self, name: &str) -> Option<&AnyField> {
        self.fields.iter().find(|f| f.name() == name)
    }

    /// The number of child fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the struct has no child fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

impl DataType<Struct> for StructType {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    fn type_name(&self) -> &str {
        "struct"
    }
}
