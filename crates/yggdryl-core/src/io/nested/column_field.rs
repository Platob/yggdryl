//! [`ColumnField`] — the **erased, recursive** field descriptor: a named, nullable column of any
//! type, *leaf* (fixed or variable) or *nested* (struct — and, in later phases, list / map).
//!
//! It is the child carrier of a [`StructField`](super::StructField) schema (a struct's children are
//! `ColumnField`s), the recursive counterpart of the flat erased [`Field`](crate::io::fixed::Field).
//! It reuses `fixed::Field` for leaves — the same one-symbol dependency the `var` family already
//! takes on `fixed::Field` — and adds a variant per nested family. A value type: it compares and
//! hashes by content, so it works as a map key and round-trips its exact logical type through Arrow.

use super::StructField;
use crate::io::fixed::Field as LeafField;
use crate::io::{DataTypeId, FieldType, Headers};

/// A **named, nullable column descriptor of any type** — the recursive, erased field. A `Leaf`
/// wraps the flat [`Field`](crate::io::fixed::Field) (every fixed / variable leaf column); a
/// `Struct` wraps a [`StructField`](super::StructField) (which itself holds child `ColumnField`s),
/// so the whole schema tree is one closed, hashable value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColumnField {
    /// A leaf column — any fixed-width or variable-length type (the flat erased field).
    Leaf(LeafField),
    /// A struct column — an ordered, named set of child fields.
    Struct(StructField),
}

impl ColumnField {
    /// A leaf field wrapping a flat [`Field`](crate::io::fixed::Field).
    pub fn leaf(field: LeafField) -> Self {
        Self::Leaf(field)
    }

    /// A struct field wrapping a [`StructField`](super::StructField).
    pub fn struct_(field: StructField) -> Self {
        Self::Struct(field)
    }

    /// The column name.
    pub fn name(&self) -> &str {
        match self {
            Self::Leaf(f) => f.name(),
            Self::Struct(f) => f.name(),
        }
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        match self {
            Self::Leaf(f) => f.nullable(),
            Self::Struct(f) => f.nullable(),
        }
    }

    /// The element type's [`DataTypeId`].
    pub fn type_id(&self) -> DataTypeId {
        match self {
            Self::Leaf(f) => FieldType::type_id(f),
            Self::Struct(_) => DataTypeId::Struct,
        }
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        match self {
            Self::Leaf(f) => f.metadata(),
            Self::Struct(f) => f.metadata(),
        }
    }

    /// A fresh field renamed to `name` — the one-line builder used to name a child within a schema.
    pub fn with_name(&self, name: &str) -> Self {
        match self {
            Self::Leaf(f) => Self::Leaf(
                LeafField::of(name, FieldType::type_id(f), f.byte_width(), f.nullable())
                    .with_metadata(f.metadata().clone()),
            ),
            Self::Struct(f) => Self::Struct(f.with_name(name)),
        }
    }

    /// Whether this field describes a nested (composite) column.
    pub fn is_nested(&self) -> bool {
        matches!(self, Self::Struct(_))
    }

    /// Whether this field describes a struct column.
    pub fn is_struct(&self) -> bool {
        matches!(self, Self::Struct(_))
    }

    /// If this is a struct field, its [`StructField`](super::StructField) — else `None`.
    pub fn as_struct(&self) -> Option<&StructField> {
        match self {
            Self::Struct(f) => Some(f),
            Self::Leaf(_) => None,
        }
    }

    /// If this is a leaf field, its flat [`Field`](crate::io::fixed::Field) — else `None`.
    pub fn as_leaf(&self) -> Option<&LeafField> {
        match self {
            Self::Leaf(f) => Some(f),
            Self::Struct(_) => None,
        }
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`) — **total** and **recursive**: a
    /// leaf maps via the flat field's exact-logical-type round-trip; a struct maps to
    /// `Field(Struct(child fields))`.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        match self {
            Self::Leaf(f) => f.to_arrow(),
            Self::Struct(f) => f.to_arrow_field(),
        }
    }

    /// Builds a field from an [`arrow_schema::Field`] (feature `arrow`), or `None` for a type this
    /// crate does not model. Recurses into a `Struct` data type; every other type is a leaf.
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        match field.data_type() {
            arrow_schema::DataType::Struct(_) => {
                StructField::from_arrow_field(field).map(Self::Struct)
            }
            _ => LeafField::from_arrow(field).map(Self::Leaf),
        }
    }
}

impl FieldType for ColumnField {
    fn name(&self) -> &str {
        self.name()
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Leaf(f) => f.type_name(),
            Self::Struct(_) => "struct",
        }
    }

    fn byte_width(&self) -> usize {
        match self {
            Self::Leaf(f) => f.byte_width(),
            Self::Struct(_) => 0,
        }
    }

    fn nullable(&self) -> bool {
        self.nullable()
    }

    fn type_id(&self) -> DataTypeId {
        self.type_id()
    }
}

impl From<LeafField> for ColumnField {
    fn from(field: LeafField) -> Self {
        Self::Leaf(field)
    }
}

impl From<StructField> for ColumnField {
    fn from(field: StructField) -> Self {
        Self::Struct(field)
    }
}
