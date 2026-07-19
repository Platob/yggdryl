//! [`StructField`] — the **centralized struct schema**: a struct column's name, nullability,
//! metadata, and the ordered child [`ColumnField`]s. It is the value-typed descriptor a
//! [`StructSerie`](super::StructSerie) reports and a [`ColumnField::Struct`](super::super::ColumnField)
//! carries. (A later phase maps it onto an Arrow `Field(Struct)` + `Schema`; kept clean but with no
//! Arrow dependency now.)

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::typed::nested::ColumnField;

/// A struct column's schema — its `name`, `nullable` flag, free-form `metadata`, and the ordered
/// `children` field descriptors. Value-typed (`Clone` / `PartialEq` / `Eq` / `Hash`) so a schema
/// keys a map, sits in a set, and travels over a wire — equal iff every field is equal, and equal
/// schemas hash equal.
///
/// ```
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{ColumnField, HeaderField, StructField};
///
/// let city = ColumnField::Leaf(HeaderField::new(Some("city"), DataTypeId::Utf8, false));
/// let zip = ColumnField::Leaf(HeaderField::new(Some("zip"), DataTypeId::I32, false));
/// let address = StructField::new(Some("address"), vec![city, zip]);
///
/// assert_eq!(address.num_fields(), 2);
/// assert_eq!(address.names(), vec!["city", "zip"]);
/// assert!(address.field_by_name("zip").is_some());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructField {
    name: Option<Box<str>>,
    nullable: bool,
    metadata: Headers,
    children: Vec<ColumnField>,
}

impl StructField {
    /// A struct schema from its `name` and ordered child field descriptors (non-nullable, no extra
    /// metadata).
    pub fn new(name: Option<&str>, children: Vec<ColumnField>) -> Self {
        StructField {
            name: name.map(Into::into),
            nullable: false,
            metadata: Headers::new(),
            children,
        }
    }

    /// Appends a child field, chainable — the schema-builder front door.
    pub fn with_child(mut self, child: ColumnField) -> Self {
        self.children.push(child);
        self
    }

    /// The child field at `index`, if present.
    pub fn field(&self, index: usize) -> Option<&ColumnField> {
        self.children.get(index)
    }

    /// The first child field named `name`, if any.
    pub fn field_by_name(&self, name: &str) -> Option<&ColumnField> {
        self.children
            .iter()
            .find(|field| field.name() == Some(name))
    }

    /// The child field names in order (an unnamed child reports `""`).
    pub fn names(&self) -> Vec<&str> {
        self.children
            .iter()
            .map(|field| field.name().unwrap_or(""))
            .collect()
    }

    /// The number of child fields.
    pub fn num_fields(&self) -> usize {
        self.children.len()
    }

    /// The child field descriptors (borrowed) — the downward edge of the recursive schema.
    pub fn children(&self) -> &[ColumnField] {
        &self.children
    }

    /// The struct's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sets the struct's name.
    pub fn set_name(&mut self, name: &str) {
        self.name = Some(name.into());
    }

    /// [`set_name`](StructField::set_name), chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.set_name(name);
        self
    }

    /// Whether the struct admits null rows.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// Sets whether the struct admits null rows.
    pub fn set_nullable(&mut self, nullable: bool) {
        self.nullable = nullable;
    }

    /// [`set_nullable`](StructField::set_nullable), chainable.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    /// The free-form metadata map (borrowed).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The free-form metadata map (mutable) — annotate the struct with any header.
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The struct's [`DataTypeId`] — always [`Struct`](DataTypeId::Struct).
    pub fn data_type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
}
