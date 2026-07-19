//! [`HeaderField`] ↔ Arrow [`Field`] — the column-descriptor bridge over the
//! [type map](super::to_arrow_data_type).
//!
//! The name, nullability, and element type (with the decimal precision·scale / fixed-size byte
//! width folded into the Arrow [`DataType`](arrow_schema::DataType)) map structurally; every other
//! free-form annotation rides along in the Arrow field's `metadata` map (and back).

use std::collections::HashMap;
use std::sync::Arc;

use arrow_schema::{DataType, Field, Fields};

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::typed::nested::{ListField, MapField, StructField};
use crate::typed::{ColumnField, Field as _, HeaderField};

use super::data_type::{from_arrow_data_type, to_arrow_data_type};

/// A [`HeaderField`] as an Arrow [`Field`] — its name, nullability, the element
/// [`DataType`](arrow_schema::DataType) from the id + params, and its free-form annotations carried
/// into the Arrow field metadata (the structural `name` / `type` / `nullable` / `precision` /
/// `scale` / `byte_width` keys are already expressed by the Arrow field, so only the extras travel
/// in `metadata`).
///
/// ```
/// use yggdryl_core::arrow::to_arrow_field;
/// use yggdryl_core::typed::HeaderField;
/// use yggdryl_core::datatype_id::DataTypeId;
/// use arrow_schema::DataType;
///
/// let field = HeaderField::new(Some("price"), DataTypeId::I64, true).with_metadata("unit", "USD");
/// let arrow = to_arrow_field(&field);
/// assert_eq!(arrow.name(), "price");
/// assert_eq!(arrow.data_type(), &DataType::Int64);
/// assert!(arrow.is_nullable());
/// assert_eq!(arrow.metadata().get("unit").map(String::as_str), Some("USD"));
/// ```
pub fn to_arrow_field(field: &HeaderField) -> Field {
    let data_type = to_arrow_data_type(
        field.data_type_id(),
        field.precision(),
        field.scale(),
        field.byte_width(),
    );
    // `name()` is total — an unnamed field carries the element type's name into Arrow.
    let mut arrow = Field::new(field.name(), data_type, field.nullable());

    // Carry the free-form annotations (everything but the structural keys) into Arrow metadata.
    let metadata = headers_to_metadata(&field.extra_annotations());
    if !metadata.is_empty() {
        arrow = arrow.with_metadata(metadata);
    }
    arrow
}

/// Every string-valued entry of a [`Headers`] map as an Arrow metadata [`HashMap`] (non-UTF-8 keys /
/// values are skipped). Shared by [`to_arrow_field`] (leaf annotations) and the nested field /
/// schema converters, so a struct / list / map's free-form metadata rides into Arrow the same way.
pub(crate) fn headers_to_metadata(headers: &Headers) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    for (name, value) in headers.iter() {
        if let (Ok(name), Ok(value)) = (core::str::from_utf8(name), core::str::from_utf8(value)) {
            metadata.insert(name.to_owned(), value.to_owned());
        }
    }
    metadata
}

/// The inverse of [`to_arrow_field`]: an Arrow [`Field`] → a [`HeaderField`] — its name,
/// nullability, the [`DataTypeId`](crate::datatype_id::DataTypeId) + params from the Arrow
/// [`DataType`](arrow_schema::DataType), and its metadata carried back as annotations.
///
/// ```
/// use yggdryl_core::arrow::from_arrow_field;
/// use yggdryl_core::typed::Field;
/// use yggdryl_core::datatype_id::DataTypeId;
/// use arrow_schema::{DataType, Field as ArrowField};
///
/// let arrow = ArrowField::new("price", DataType::Int64, true);
/// let field = from_arrow_field(&arrow);
/// assert_eq!(field.name(), "price");
/// assert_eq!(field.data_type_id(), DataTypeId::I64);
/// assert!(field.nullable());
/// ```
pub fn from_arrow_field(field: &Field) -> HeaderField {
    let (id, precision, scale, byte_width) = from_arrow_data_type(field.data_type());
    let mut header = HeaderField::new(Some(field.name()), id, field.is_nullable());
    if let Some(precision) = precision {
        header.headers_mut().set_precision(precision);
    }
    if let Some(scale) = scale {
        header.headers_mut().set_scale(scale);
    }
    if let Some(byte_width) = byte_width {
        header.headers_mut().set_byte_width(byte_width);
    }
    for (key, value) in field.metadata() {
        header.set_metadata(key, value);
    }
    header
}

// ---- nested (recursive) ColumnField <-> Arrow Field ------------------------------------------

/// A [`ColumnField`] as an Arrow [`Field`], recursing through arbitrary nesting: a
/// [`Leaf`](ColumnField::Leaf) is [`to_arrow_field`]; a [`Struct`](ColumnField::Struct) becomes a
/// `Field(Struct)` over its children; a [`List`](ColumnField::List) a `Field(List)` over its item;
/// a [`Map`](ColumnField::Map) a `Field(Map)` over an `entries` `Struct(key, value)` carrying the
/// `keys_sorted` flag. Every nested field's `name` / `nullable` / free-form `metadata` ride along.
pub(crate) fn column_field_to_arrow(field: &ColumnField) -> Field {
    match field {
        ColumnField::Leaf(header) => to_arrow_field(header),
        ColumnField::Struct(struct_field) => {
            let children: Vec<Field> = struct_field
                .children()
                .iter()
                .map(column_field_to_arrow)
                .collect();
            let data_type = DataType::Struct(Fields::from(children));
            with_field_metadata(
                Field::new(
                    struct_field.name().unwrap_or_default(),
                    data_type,
                    struct_field.nullable(),
                ),
                struct_field.metadata(),
            )
        }
        ColumnField::List(list_field) => {
            let item = column_field_to_arrow(list_field.item());
            let data_type = DataType::List(Arc::new(item));
            with_field_metadata(
                Field::new(
                    list_field.name().unwrap_or_default(),
                    data_type,
                    list_field.nullable(),
                ),
                list_field.metadata(),
            )
        }
        ColumnField::Map(map_field) => {
            // A map's `key` field is always non-nullable (Arrow forbids null map keys).
            let key = force_non_nullable(column_field_to_arrow(map_field.key()));
            let value = column_field_to_arrow(map_field.value());
            let entries = Fields::from(vec![key, value]);
            let entries_field = Arc::new(Field::new("entries", DataType::Struct(entries), false));
            let data_type = DataType::Map(entries_field, map_field.keys_sorted());
            with_field_metadata(
                Field::new(
                    map_field.name().unwrap_or_default(),
                    data_type,
                    map_field.nullable(),
                ),
                map_field.metadata(),
            )
        }
    }
}

/// The inverse of [`column_field_to_arrow`]: an Arrow [`Field`] → a [`ColumnField`], recursing on the
/// Arrow [`DataType`]. A `Struct` / `List` (+ `LargeList` / `FixedSizeList`) / `Map` becomes the
/// matching nested field; any other type is a leaf [`from_arrow_field`]. `keys_sorted` and every
/// field's `name` / `nullable` / `metadata` are restored.
pub(crate) fn column_field_from_arrow(field: &Field) -> ColumnField {
    match field.data_type() {
        DataType::Struct(fields) => {
            let children = fields
                .iter()
                .map(|child| column_field_from_arrow(child))
                .collect();
            let mut struct_field = StructField::new(Some(field.name()), children);
            struct_field.set_nullable(field.is_nullable());
            copy_metadata(struct_field.metadata_mut(), field);
            ColumnField::Struct(struct_field)
        }
        DataType::List(item) | DataType::LargeList(item) | DataType::FixedSizeList(item, _) => {
            let mut list_field = ListField::new(Some(field.name()), column_field_from_arrow(item));
            list_field.set_nullable(field.is_nullable());
            copy_metadata(list_field.metadata_mut(), field);
            ColumnField::List(list_field)
        }
        DataType::Map(entries, keys_sorted) => {
            let (key, value) = match entries.data_type() {
                DataType::Struct(fields) if fields.len() >= 2 => (
                    column_field_from_arrow(&fields[0]),
                    column_field_from_arrow(&fields[1]),
                ),
                _ => (
                    ColumnField::Leaf(HeaderField::new(Some("key"), DataTypeId::Unknown, false)),
                    ColumnField::Leaf(HeaderField::new(Some("value"), DataTypeId::Unknown, true)),
                ),
            };
            let mut map_field = MapField::new(Some(field.name()), key, value);
            map_field.set_nullable(field.is_nullable());
            map_field.set_keys_sorted(*keys_sorted);
            copy_metadata(map_field.metadata_mut(), field);
            ColumnField::Map(map_field)
        }
        _ => ColumnField::Leaf(from_arrow_field(field)),
    }
}

/// Attaches a nested field's free-form [`Headers`] metadata to its Arrow [`Field`] (a no-op when
/// empty).
fn with_field_metadata(field: Field, metadata: &Headers) -> Field {
    let metadata = headers_to_metadata(metadata);
    if metadata.is_empty() {
        field
    } else {
        field.with_metadata(metadata)
    }
}

/// Copies an Arrow [`Field`]'s metadata into a nested field's [`Headers`] map. Arrow field metadata
/// is an unordered `HashMap`, so insertion order is not preserved on the round-trip (a documented
/// edge — the values round-trip, their relative order may not).
fn copy_metadata(target: &mut Headers, field: &Field) {
    for (key, value) in field.metadata() {
        target.insert(key, value);
    }
}

/// The same Arrow [`Field`] forced non-nullable — used for a map's `key` field (Arrow forbids null
/// map keys). Preserves the name, element type, and metadata.
pub(crate) fn force_non_nullable(field: Field) -> Field {
    if !field.is_nullable() {
        return field;
    }
    Field::new(field.name(), field.data_type().clone(), false)
        .with_metadata(field.metadata().clone())
}
