//! [`ColumnField`] — the **recursive field descriptor** of a [`Column`](super::Column): a flat
//! column's [`HeaderField`] or a nested struct's [`StructField`]. The schema counterpart of the
//! erased [`Value`](super::Value) — where `Value` is one element, `ColumnField` is one column's type.

use crate::datatype_id::DataTypeId;
use crate::typed::nested::StructField;
use crate::typed::{Field, HeaderField};

/// A column's field — [`Leaf`](ColumnField::Leaf) for a flat column (carrying its [`HeaderField`]) or
/// [`Struct`](ColumnField::Struct) for a nested struct (carrying its [`StructField`], and through it
/// the child fields). Value-typed (`Clone` / `PartialEq` / `Eq` / `Hash`) — the byte-canonical
/// identity every descriptor in the crate shares, so a field keys a map, sits in a set, and travels
/// over a wire.
///
// DESIGN: `List` / `Map` field variants land with their carriers in a later phase; the enum is
// `#[non_exhaustive]` so adding them stays additive (downstream matches keep a wildcard arm).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColumnField {
    /// A flat column's field — its name, element [`DataTypeId`], nullability, and any annotations.
    Leaf(HeaderField),
    /// A nested struct column's field — its schema, including the ordered child fields.
    Struct(StructField),
}

impl ColumnField {
    /// The column name, if set.
    pub fn name(&self) -> Option<&str> {
        match self {
            ColumnField::Leaf(field) => field.name(),
            ColumnField::Struct(field) => field.name(),
        }
    }

    /// The column's element [`DataTypeId`] — the leaf's declared type, or
    /// [`Struct`](DataTypeId::Struct) for a nested struct.
    pub fn data_type_id(&self) -> DataTypeId {
        match self {
            ColumnField::Leaf(field) => field.data_type_id(),
            ColumnField::Struct(_) => DataTypeId::Struct,
        }
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        match self {
            ColumnField::Leaf(field) => field.nullable(),
            ColumnField::Struct(field) => field.nullable(),
        }
    }

    /// The child field descriptors — **empty** for a [`Leaf`](ColumnField::Leaf), the struct's child
    /// fields for a [`Struct`](ColumnField::Struct). The downward edge of the recursive schema.
    pub fn children(&self) -> &[ColumnField] {
        match self {
            ColumnField::Leaf(_) => &[],
            ColumnField::Struct(field) => field.children(),
        }
    }
}
