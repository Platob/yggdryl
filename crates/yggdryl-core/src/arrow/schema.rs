//! [`StructField`] ↔ Arrow [`Schema`](arrow_schema::Schema) — the top-level struct-schema bridge.
//!
//! A struct's ordered child [`ColumnField`](crate::typed::ColumnField)s become the schema's
//! [`Fields`](arrow_schema::Fields) (each through the recursive
//! [`column_field_to_arrow`](super::field) map), and the struct-level free-form metadata rides onto
//! the schema metadata. The inverse rebuilds the [`StructField`] from the schema's fields + metadata.
//! A [`Schema`](arrow_schema::Schema) carries no *name* (a struct's name is dropped) and its metadata
//! is an unordered map (insertion order is not preserved) — the only edges of this otherwise exact
//! round-trip.

use arrow_schema::Schema;

use crate::typed::nested::StructField;

use super::field::{column_field_from_arrow, column_field_to_arrow, headers_to_metadata};

/// A [`StructField`] as an Arrow [`Schema`]: its child fields through the recursive
/// [`column_field_to_arrow`](super::field) map, carrying the struct-level metadata onto the schema.
///
/// ```
/// use yggdryl_core::arrow::struct_field_to_arrow_schema;
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{ColumnField, HeaderField, StructField};
/// use arrow_schema::DataType;
///
/// let id = ColumnField::Leaf(HeaderField::new(Some("id"), DataTypeId::I64, false));
/// let name = ColumnField::Leaf(HeaderField::new(Some("name"), DataTypeId::Utf8, true));
/// let schema = struct_field_to_arrow_schema(&StructField::new(Some("person"), vec![id, name]));
///
/// assert_eq!(schema.fields().len(), 2);
/// assert_eq!(schema.field(0).name(), "id");
/// assert_eq!(schema.field(0).data_type(), &DataType::Int64);
/// assert_eq!(schema.field(1).data_type(), &DataType::Utf8);
/// ```
pub fn struct_field_to_arrow_schema(field: &StructField) -> Schema {
    let fields: Vec<_> = field.children().iter().map(column_field_to_arrow).collect();
    let schema = Schema::new(fields);
    let metadata = headers_to_metadata(field.metadata());
    if metadata.is_empty() {
        schema
    } else {
        schema.with_metadata(metadata)
    }
}

/// The inverse of [`struct_field_to_arrow_schema`]: an Arrow [`Schema`] → a [`StructField`] — its
/// fields rebuilt through the recursive [`column_field_from_arrow`](super::field) map and the schema
/// metadata carried back. The rebuilt struct is unnamed (a schema has no name).
///
/// ```
/// use yggdryl_core::arrow::{struct_field_from_arrow_schema, struct_field_to_arrow_schema};
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{ColumnField, HeaderField, StructField};
///
/// let id = ColumnField::Leaf(HeaderField::new(Some("id"), DataTypeId::I64, false));
/// let schema = struct_field_to_arrow_schema(&StructField::new(None, vec![id]));
///
/// let field = struct_field_from_arrow_schema(&schema);
/// assert_eq!(field.num_fields(), 1);
/// assert_eq!(field.names(), vec!["id"]);
/// assert_eq!(field.field(0).unwrap().data_type_id(), DataTypeId::I64);
/// ```
pub fn struct_field_from_arrow_schema(schema: &Schema) -> StructField {
    let children = schema
        .fields()
        .iter()
        .map(|field| column_field_from_arrow(field))
        .collect();
    let mut struct_field = StructField::new(None, children);
    for (key, value) in schema.metadata() {
        struct_field.metadata_mut().insert(key, value);
    }
    struct_field
}
