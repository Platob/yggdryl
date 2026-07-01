//! Tests for the schema layer's Arrow interop: `DataTypeId` format strings and the
//! primitive types'/fields' scalar round-trip. The dynamic / nested Arrow round-trip
//! is tested in the `yggdryl-scalar` crate.

use yggdryl_schema::{
    ArrowArray, ArrowError, ArrowSchema, DataType, DataTypeId, Field, Int128Type, Int32Type,
    Int64Field, Metadata, UInt256Field, UInt256Type,
};

/// The Arrow metadata key an extension type records its name under.
const EXT_KEY: &[u8] = b"ARROW:extension:name";

fn metadata(pairs: &[(&str, &str)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect()
}

#[test]
fn primitive_types_carry_arrow_format_strings() {
    // The format string lives on the shared discriminant, so every typed `DataType`
    // reaches it through `type_id()`.
    assert_eq!(Int32Type::new().type_id().arrow_format(), "i");
    assert_eq!(DataTypeId::UInt64.arrow_format(), "L");
    assert_eq!(DataTypeId::Utf8.arrow_format(), "u");
    assert_eq!(DataTypeId::Struct.arrow_format(), "+s");
    // The wide integers borrow FixedSizeBinary and are tagged by extension name.
    assert_eq!(Int128Type::new().type_id().arrow_format(), "w:16");
    assert_eq!(UInt256Type::new().type_id().arrow_format(), "w:32");
    assert_eq!(
        DataTypeId::Int128.arrow_extension_name(),
        Some("yggdryl.int128")
    );
    assert_eq!(DataTypeId::Int32.arrow_extension_name(), None);
}

#[test]
fn scalar_type_round_trips() {
    let node = Int32Type::new().to_arrow_scalar();
    assert_eq!(node.format(), "i");
    assert_eq!(node.primitive_id().unwrap(), DataTypeId::Int32);
    assert_eq!(
        Int32Type::from_arrow_scalar(&node).unwrap(),
        Int32Type::new()
    );

    // A wide integer carries its extension type through the scalar path too.
    let wide = Int128Type::new().to_arrow_scalar();
    assert_eq!(wide.format(), "w:16");
    assert!(wide.metadata().contains_key(EXT_KEY));
    assert_eq!(
        Int128Type::from_arrow_scalar(&wide).unwrap(),
        Int128Type::new()
    );
}

#[test]
fn scalar_field_round_trips_with_attributes() {
    let field = UInt256Field::new("amount")
        .with_nullable(true)
        .with_metadata(metadata(&[("unit", "wei")]));
    let node = field.to_arrow_scalar();
    assert_eq!(node.name(), "amount");
    assert!(node.nullable());
    assert!(node.metadata().contains_key(EXT_KEY));

    let rebuilt = UInt256Field::from_arrow_scalar(&node).unwrap();
    assert_eq!(rebuilt.name(), "amount");
    assert!(rebuilt.nullable());
    assert_eq!(rebuilt.dtype().type_id(), DataTypeId::UInt256);
    assert_eq!(rebuilt.metadata(), Some(&metadata(&[("unit", "wei")])));
}

#[test]
fn scalar_from_arrow_rejects_a_mismatched_type() {
    // A utf8 type node decoded as Int32 → mismatch.
    let utf8 = ArrowSchema::primitive(DataTypeId::Utf8);
    assert_eq!(
        Int32Type::from_arrow_scalar(&utf8),
        Err(ArrowError::TypeMismatch {
            expected: DataTypeId::Int32,
            found: DataTypeId::Utf8,
        })
    );

    let int32_node = Int32Type::new().to_arrow_scalar();
    assert_eq!(
        Int64Field::from_arrow_scalar(&int32_node),
        Err(ArrowError::TypeMismatch {
            expected: DataTypeId::Int64,
            found: DataTypeId::Int32,
        })
    );
}

#[test]
fn scalar_from_arrow_rejects_a_struct_node() {
    // The primitive decoders can't resolve a struct node — that's the dynamic layer.
    let struct_node = ArrowSchema::from_parts(
        "+s".to_string(),
        String::new(),
        false,
        Metadata::new(),
        vec![],
    );
    assert_eq!(
        Int32Type::from_arrow_scalar(&struct_node),
        Err(ArrowError::UnsupportedFormat("+s".to_string()))
    );
}

#[test]
fn concrete_field_from_arrow_array_sets_nullable() {
    let schema = Int64Field::new("count").to_arrow_scalar();
    let field =
        Int64Field::from_arrow_array(&schema, &ArrowArray::from_parts(3, 2, vec![])).unwrap();
    assert_eq!(field.name(), "count");
    assert!(field.nullable());

    // A mismatched type is still rejected.
    let utf8 = ArrowSchema::primitive(DataTypeId::Utf8);
    assert_eq!(
        Int64Field::from_arrow_array(&utf8, &ArrowArray::from_parts(0, 0, vec![])),
        Err(ArrowError::TypeMismatch {
            expected: DataTypeId::Int64,
            found: DataTypeId::Utf8,
        })
    );
}
