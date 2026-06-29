//! Cross-cutting tests for the data-type, scalar and field layers.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::{
    AnyField, AnyType, Binary, BinaryBased, BinaryScalar, DataType, Field, PrimitiveField, Scalar,
    StringScalar, TypeCategory, TypeError, Utf8,
};

#[test]
fn datatype_string_round_trips() {
    for (name, expected) in [
        ("binary", AnyType::Binary(Binary::new())),
        ("large_binary", AnyType::Binary(Binary::large())),
        ("string", AnyType::Utf8(Utf8::new())),
        ("large_string", AnyType::Utf8(Utf8::large())),
    ] {
        let parsed = AnyType::from_str(name).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.to_str(), name);
        assert_eq!(AnyType::from_bytes(&parsed.to_bytes()).unwrap(), parsed);
        assert_eq!(AnyType::from_mapping(&parsed.to_mapping()).unwrap(), parsed);
    }
}

#[test]
fn datatype_aliases_and_errors() {
    assert_eq!(Utf8::from_str("utf8").unwrap(), Utf8::new());
    assert_eq!(Utf8::from_str("large_utf8").unwrap(), Utf8::large());
    assert!(matches!(
        AnyType::from_str("flob"),
        Err(TypeError::UnknownType(_))
    ));
}

#[test]
fn datatype_categories_and_flags() {
    assert_eq!(Binary::new().category(), TypeCategory::Primitive);
    assert!(!Binary::new().is_utf8());
    assert!(Binary::large().is_large());
    assert!(Utf8::new().is_utf8());
    assert!(!Utf8::new().is_large());
}

#[test]
fn datatype_is_hashable() {
    let mut counts: HashMap<AnyType, u32> = HashMap::new();
    *counts.entry(Binary::new().to_any()).or_default() += 1;
    *counts.entry(Binary::new().to_any()).or_default() += 1;
    *counts.entry(Utf8::new().to_any()).or_default() += 1;
    assert_eq!(counts[&AnyType::Binary(Binary::new())], 2);
    assert_eq!(counts[&AnyType::Utf8(Utf8::new())], 1);
}

#[test]
fn binary_scalar_borrows_without_copy() {
    let scalar = BinaryScalar::new(vec![0u8, 1, 2, 3]);
    assert_eq!(scalar.as_bytes(), Some([0u8, 1, 2, 3].as_slice()));
    assert_eq!(scalar.len(), Some(4));
    assert!(!scalar.is_null());
    assert_eq!(scalar.data_type(), AnyType::Binary(Binary::new()));
    assert!(BinaryScalar::null().is_null());
    assert_eq!(BinaryScalar::null().as_bytes(), None);
}

#[test]
fn string_scalar_validates_utf8() {
    let scalar = StringScalar::new("yggdryl");
    assert_eq!(scalar.as_str(), Some("yggdryl"));
    assert_eq!(scalar.as_bytes(), Some("yggdryl".as_bytes()));
    assert_eq!(scalar.data_type(), AnyType::Utf8(Utf8::new()));

    let bad = crate::Buffer::from_slice(&[0xff, 0xfe]);
    assert!(StringScalar::from_buffer(bad).is_err());
}

#[test]
fn field_round_trips_with_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("unit".to_string(), "bytes".to_string());
    let field = Field::new("payload", Binary::large().to_any(), false).with_metadata(metadata);

    let mapping = field.to_mapping();
    assert_eq!(mapping["name"], "payload");
    assert_eq!(mapping["type"], "large_binary");
    assert_eq!(mapping["nullable"], "false");
    assert_eq!(mapping["metadata.unit"], "bytes");

    assert_eq!(AnyField::from_mapping(&mapping).unwrap(), field);
    assert_eq!(AnyField::from_bytes(&field.to_bytes()).unwrap(), field);
}

#[test]
fn field_nullable_defaults_to_true() {
    let mut mapping = BTreeMap::new();
    mapping.insert("name".to_string(), "id".to_string());
    mapping.insert("type".to_string(), "string".to_string());
    let field = AnyField::from_mapping(&mapping).unwrap();
    assert!(field.is_nullable());
}

#[test]
fn typed_field_is_a_primitive_field() {
    fn assert_primitive<F: PrimitiveField>(_: &F) {}
    let field = Field::new("x", Binary::new(), false);
    assert_primitive(&field); // compile-time proof Field<Binary>: PrimitiveField
    assert_eq!(field.to_any().data_type().to_str(), "binary");
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trips_through_json() {
    let ty = AnyType::Utf8(Utf8::large());
    let json = serde_json::to_string(&ty).unwrap();
    assert_eq!(json, "\"large_string\"");
    assert_eq!(serde_json::from_str::<AnyType>(&json).unwrap(), ty);

    let field = Field::new("c", Binary::new().to_any(), true);
    let json = serde_json::to_string(&field).unwrap();
    assert_eq!(serde_json::from_str::<AnyField>(&json).unwrap(), field);

    let scalar = StringScalar::new("hi");
    let json = serde_json::to_string(&scalar).unwrap();
    assert_eq!(serde_json::from_str::<StringScalar>(&json).unwrap(), scalar);
}

#[cfg(feature = "serde")]
#[test]
fn deserializing_invalid_utf8_string_scalar_fails() {
    // A string scalar whose bytes are not UTF-8 must be rejected so `as_str`
    // stays sound.
    let json = r#"{"type":"string","value":[255,254]}"#;
    assert!(serde_json::from_str::<StringScalar>(json).is_err());
}

#[cfg(feature = "json")]
#[test]
fn json_helpers_round_trip() {
    let ty = AnyType::Binary(Binary::large());
    assert_eq!(AnyType::from_json(&ty.to_json()).unwrap(), ty);

    let field = Field::new("c", Utf8::new().to_any(), false);
    assert_eq!(AnyField::from_json(&field.to_json()).unwrap(), field);

    let scalar = BinaryScalar::new(vec![9u8, 9, 9]);
    assert_eq!(BinaryScalar::from_json(&scalar.to_json()).unwrap(), scalar);
}
