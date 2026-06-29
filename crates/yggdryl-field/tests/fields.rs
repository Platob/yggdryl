//! Tests for the Arrow field layer.

use std::collections::BTreeMap;

use yggdryl_dtype::{BinaryType, DataType, Utf8Type};
use yggdryl_field::{AnyField, Field, PrimitiveField};

#[test]
fn field_round_trips_with_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("unit".to_string(), "bytes".to_string());
    let field = Field::new("payload", BinaryType::large().to_any(), false).with_metadata(metadata);

    let mapping = field.to_mapping();
    assert_eq!(mapping["type"], "large_binary");
    assert_eq!(AnyField::from_mapping(&mapping).unwrap(), field);
    assert_eq!(AnyField::from_bytes(&field.to_bytes()).unwrap(), field);
}

#[test]
fn field_nullable_defaults_to_true() {
    let mut mapping = BTreeMap::new();
    mapping.insert("name".to_string(), "id".to_string());
    mapping.insert("type".to_string(), "string".to_string());
    assert!(AnyField::from_mapping(&mapping).unwrap().is_nullable());
}

#[test]
fn typed_field_is_a_primitive_field() {
    fn assert_primitive<F: PrimitiveField>(_: &F) {}
    let field = Field::new("x", Utf8Type::new(), false);
    assert_primitive(&field); // compile-time proof Field<Utf8Type>: PrimitiveField
    assert_eq!(field.to_any().data_type().to_str(), "string");
}

#[cfg(feature = "json")]
#[test]
fn field_json_round_trips() {
    use yggdryl_core::Jsonable;

    let field = Field::new("c", Utf8Type::new().to_any(), false);
    assert_eq!(AnyField::from_json(&field.to_json()).unwrap(), field);
    assert_eq!(AnyField::from_bson(&field.to_bson()).unwrap(), field);
}
