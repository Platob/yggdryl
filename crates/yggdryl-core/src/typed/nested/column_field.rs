//! [`ColumnField`] — the **recursive field descriptor** of a [`Column`](super::Column): a flat
//! column's [`HeaderField`] or a nested struct's [`StructField`]. The schema counterpart of the
//! erased [`Value`](super::Value) — where `Value` is one element, `ColumnField` is one column's type.

use crate::datatype_id::DataTypeId;
use crate::typed::nested::{ListField, MapField, StructField};
use crate::typed::{Field, HeaderField};

/// A column's field — [`Leaf`](ColumnField::Leaf) for a flat column (carrying its [`HeaderField`]),
/// [`Struct`](ColumnField::Struct) for a nested struct (carrying its [`StructField`], and through it
/// the child fields), [`List`](ColumnField::List) for a list (its [`ListField`] + item), or
/// [`Map`](ColumnField::Map) for a map (its [`MapField`] + key / value). Value-typed (`Clone` /
/// `PartialEq` / `Eq` / `Hash`) — the byte-canonical identity every descriptor in the crate shares,
/// so a field keys a map, sits in a set, and travels over a wire.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ColumnField {
    /// A flat column's field — its name, element [`DataTypeId`], nullability, and any annotations.
    Leaf(HeaderField),
    /// A nested struct column's field — its schema, including the ordered child fields.
    Struct(StructField),
    /// A list column's field — its schema, including the child **item** field.
    List(ListField),
    /// A map column's field — its schema, including the child **key** and **value** fields.
    Map(MapField),
}

impl ColumnField {
    /// The **explicitly stored** column name, if set (`None` for an unnamed field). For a
    /// [`Leaf`](ColumnField::Leaf) this is the field's raw stored [`X-Name`](crate::headers::Headers::NAME),
    /// not the dtype-name default of [`Field::name`].
    pub fn name(&self) -> Option<&str> {
        match self {
            ColumnField::Leaf(field) => field.headers().name(),
            ColumnField::Struct(field) => field.name(),
            ColumnField::List(field) => field.name(),
            ColumnField::Map(field) => field.name(),
        }
    }

    /// The column's element [`DataTypeId`] — the leaf's declared type, or the nested type
    /// ([`Struct`](DataTypeId::Struct) / [`List`](DataTypeId::List) / [`Map`](DataTypeId::Map)).
    pub fn data_type_id(&self) -> DataTypeId {
        match self {
            ColumnField::Leaf(field) => field.data_type_id(),
            ColumnField::Struct(_) => DataTypeId::Struct,
            ColumnField::List(_) => DataTypeId::List,
            ColumnField::Map(_) => DataTypeId::Map,
        }
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        match self {
            ColumnField::Leaf(field) => field.nullable(),
            ColumnField::Struct(field) => field.nullable(),
            ColumnField::List(field) => field.nullable(),
            ColumnField::Map(field) => field.nullable(),
        }
    }

    /// The child field descriptors — the downward edge of the recursive schema. **Empty** for a
    /// [`Leaf`](ColumnField::Leaf); the struct's ordered child fields for a
    /// [`Struct`](ColumnField::Struct); the single **item** field for a [`List`](ColumnField::List);
    /// and the **key** then **value** fields for a [`Map`](ColumnField::Map).
    ///
    // DESIGN: returns an owned `Vec` (not a borrowed slice) because a list holds one boxed item and a
    // map holds two separately-boxed key / value fields — neither is a contiguous `[ColumnField]` to
    // borrow — so the uniform shape is a small, on-demand collection (a leaf allocates nothing).
    pub fn children(&self) -> Vec<ColumnField> {
        match self {
            ColumnField::Leaf(_) => Vec::new(),
            ColumnField::Struct(field) => field.children().to_vec(),
            ColumnField::List(field) => vec![field.item().clone()],
            ColumnField::Map(field) => vec![field.key().clone(), field.value().clone()],
        }
    }
}
