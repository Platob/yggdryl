//! [`HeaderField`] ↔ Arrow [`Field`] — the column-descriptor bridge over the
//! [type map](super::to_arrow_data_type).
//!
//! The name, nullability, and element type (with the decimal precision·scale / fixed-size byte
//! width folded into the Arrow [`DataType`](arrow_schema::DataType)) map structurally; every other
//! free-form annotation rides along in the Arrow field's `metadata` map (and back).

use std::collections::HashMap;

use arrow_schema::Field;

use crate::typed::{Field as _, HeaderField};

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
    let mut arrow = Field::new(
        field.name().unwrap_or_default(),
        data_type,
        field.nullable(),
    );

    // Carry the free-form annotations (everything but the structural keys) into Arrow metadata.
    let extra = field.extra_annotations();
    if !extra.is_empty() {
        let mut metadata = HashMap::new();
        for (name, value) in extra.iter() {
            if let (Ok(name), Ok(value)) = (core::str::from_utf8(name), core::str::from_utf8(value))
            {
                metadata.insert(name.to_owned(), value.to_owned());
            }
        }
        if !metadata.is_empty() {
            arrow = arrow.with_metadata(metadata);
        }
    }
    arrow
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
/// assert_eq!(field.name(), Some("price"));
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
