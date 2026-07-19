//! [`ListField`] — the **list schema**: a list column's name, nullability, metadata, and the single
//! child [`ColumnField`] describing its **item** element type. It is the value-typed descriptor a
//! [`ListSerie`](super::ListSerie) reports and a [`ColumnField::List`](super::super::ColumnField)
//! carries. (A later phase maps it onto an Arrow `Field(List)`; kept clean but with no Arrow
//! dependency now.)

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::typed::nested::ColumnField;

/// A list column's schema — its `name`, `nullable` flag, free-form `metadata`, and the single
/// `item` field describing every element's type (a list *of* that item). Value-typed (`Clone` /
/// `PartialEq` / `Eq` / `Hash`) so a schema keys a map, sits in a set, and travels over a wire —
/// equal iff the name, nullability, metadata, and item all match, and equal schemas hash equal.
///
/// ```
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{ColumnField, HeaderField, ListField};
///
/// let item = ColumnField::Leaf(HeaderField::new(Some("item"), DataTypeId::I64, true));
/// let field = ListField::new(Some("scores"), item);
///
/// assert_eq!(field.name(), Some("scores"));
/// assert_eq!(field.data_type_id(), DataTypeId::List);
/// assert_eq!(field.item().data_type_id(), DataTypeId::I64);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ListField {
    name: Option<Box<str>>,
    nullable: bool,
    metadata: Headers,
    item: Box<ColumnField>,
}

impl ListField {
    /// A list schema from its `name` and the child `item` field (non-nullable, no extra metadata).
    pub fn new(name: Option<&str>, item: ColumnField) -> Self {
        ListField {
            name: name.map(Into::into),
            nullable: false,
            metadata: Headers::new(),
            item: Box::new(item),
        }
    }

    /// The child **item** field describing every element's type — the downward edge of the schema.
    pub fn item(&self) -> &ColumnField {
        &self.item
    }

    /// Replaces the child **item** field, chainable — the schema-builder front door.
    pub fn with_item(mut self, item: ColumnField) -> Self {
        self.item = Box::new(item);
        self
    }

    /// The list's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sets the list's name.
    pub fn set_name(&mut self, name: &str) {
        self.name = Some(name.into());
    }

    /// [`set_name`](ListField::set_name), chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.set_name(name);
        self
    }

    /// Whether the list admits null elements.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// Sets whether the list admits null elements.
    pub fn set_nullable(&mut self, nullable: bool) {
        self.nullable = nullable;
    }

    /// [`set_nullable`](ListField::set_nullable), chainable.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    /// The free-form metadata map (borrowed).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The free-form metadata map (mutable) — annotate the list with any header.
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The list's [`DataTypeId`] — always [`List`](DataTypeId::List).
    pub fn data_type_id(&self) -> DataTypeId {
        DataTypeId::List
    }
}
