//! Tests for Apache Arrow interoperability: every `DataType` / `Field` encodes to an
//! [`ArrowSchema`] node and rebuilds from one losslessly.

use yggdryl_schema::{
    AnyField, AnyType, ArrowError, ArrowSchema, DataType, DataTypeId, Field, Int128Type, Int32Type,
    Int64Field, Metadata, StructField, StructType, UInt256Field, UInt256Type,
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
fn primitive_type_round_trips() {
    for id in [
        DataTypeId::Int8,
        DataTypeId::Int64,
        DataTypeId::UInt16,
        DataTypeId::Utf8,
    ] {
        let ty = AnyType::primitive(id);
        let node = ty.to_arrow();
        assert_eq!(node.format(), id.arrow_format());
        assert!(node.metadata().is_empty()); // no extension for native types
        assert_eq!(AnyType::from_arrow(&node).unwrap(), ty);
    }
}

#[test]
fn wide_integer_uses_an_extension_type() {
    let ty = AnyType::primitive(DataTypeId::Int128);
    let node = ty.to_arrow();
    assert_eq!(node.format(), "w:16");
    // The extension name rides in the node metadata under the Arrow key.
    assert_eq!(
        node.metadata().get(EXT_KEY).map(Vec::as_slice),
        Some(b"yggdryl.int128".as_slice())
    );
    assert_eq!(AnyType::from_arrow(&node).unwrap(), ty);
}

#[test]
fn field_round_trips_with_nullability_and_metadata() {
    let field = AnyField::from_parts(
        "amount".to_string(),
        AnyType::primitive(DataTypeId::UInt256),
        true,
        Some(metadata(&[("unit", "wei")])),
    );
    let node = field.to_arrow();
    assert_eq!(node.name(), "amount");
    assert!(node.nullable());
    // Both the extension name and the user metadata are present on the node.
    assert!(node.metadata().contains_key(EXT_KEY));
    assert_eq!(
        node.metadata().get(b"unit".as_slice()).map(Vec::as_slice),
        Some(b"wei".as_slice())
    );
    // Rebuilt field is identical — the internal extension key is stripped back out.
    let rebuilt = AnyField::from_arrow(&node).unwrap();
    assert_eq!(rebuilt, field);
    assert_eq!(rebuilt.metadata(), Some(&metadata(&[("unit", "wei")])));
}

#[test]
fn field_without_metadata_round_trips_to_none() {
    let field = AnyField::new("id", AnyType::primitive(DataTypeId::Int32));
    let rebuilt = AnyField::from_arrow(&field.to_arrow()).unwrap();
    assert_eq!(rebuilt, field);
    assert_eq!(rebuilt.metadata(), None); // empty node metadata → None, not Some({})
}

#[test]
fn struct_schema_round_trips_recursively() {
    let inner = AnyType::struct_type(StructType::new(vec![
        AnyField::new("x", AnyType::primitive(DataTypeId::Int32)),
        AnyField::new("y", AnyType::primitive(DataTypeId::Int32)),
    ]));
    let schema = StructField::new(
        "record",
        vec![
            AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
            AnyField::new("big", AnyType::primitive(DataTypeId::Int128)),
            AnyField::new("point", inner),
        ],
    )
    .with_nullable(true)
    .with_metadata(metadata(&[("origin", "test")]));

    let node = schema.to_arrow();
    assert_eq!(node.format(), "+s");
    assert_eq!(node.children().len(), 3);
    // The nested struct is itself a "+s" node with its own children.
    assert_eq!(node.children()[2].format(), "+s");
    assert_eq!(node.children()[2].children().len(), 2);

    assert_eq!(StructField::from_arrow(&node).unwrap(), schema);
}

#[test]
fn from_arrow_rejects_unmodelled_and_malformed_nodes() {
    // An unknown format string.
    let bogus = ArrowSchema::from_parts(
        "z".to_string(),
        String::new(),
        false,
        Metadata::new(),
        Vec::new(),
    );
    assert_eq!(
        AnyType::from_arrow(&bogus),
        Err(ArrowError::UnsupportedFormat("z".to_string()))
    );

    // A FixedSizeBinary with no extension name — a type we cannot resolve.
    let untagged = ArrowSchema::from_parts(
        "w:16".to_string(),
        String::new(),
        false,
        Metadata::new(),
        Vec::new(),
    );
    assert_eq!(
        AnyType::from_arrow(&untagged),
        Err(ArrowError::MissingExtension("w:16".to_string()))
    );

    // A FixedSizeBinary tagged with an extension we don't know.
    let mut foreign_meta = Metadata::new();
    foreign_meta.insert(EXT_KEY.to_vec(), b"arrow.uuid".to_vec());
    let foreign = ArrowSchema::from_parts(
        "w:16".to_string(),
        String::new(),
        false,
        foreign_meta,
        Vec::new(),
    );
    assert_eq!(
        AnyType::from_arrow(&foreign),
        Err(ArrowError::UnknownExtension("arrow.uuid".to_string()))
    );

    // Asking for a struct from a non-struct node.
    let scalar = AnyType::primitive(DataTypeId::Int32).to_arrow();
    assert_eq!(
        StructField::from_arrow(&scalar),
        Err(ArrowError::NotAStruct("i".to_string()))
    );
}

#[test]
fn scalar_type_round_trips() {
    let node = Int32Type::new().to_arrow_scalar();
    assert_eq!(node.format(), "i");
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
    // A type asked to decode a different type's node.
    let utf8 = AnyType::primitive(DataTypeId::Utf8).to_arrow();
    assert_eq!(
        Int32Type::from_arrow_scalar(&utf8),
        Err(ArrowError::TypeMismatch {
            expected: DataTypeId::Int32,
            found: DataTypeId::Utf8,
        })
    );

    // A field asked to decode a node of the wrong scalar type. (Concrete fields are
    // not `PartialEq`, so compare the error rather than the whole `Result`.)
    let int32_node = Int32Type::new().to_arrow_scalar();
    assert_eq!(
        Int64Field::from_arrow_scalar(&int32_node).unwrap_err(),
        ArrowError::TypeMismatch {
            expected: DataTypeId::Int64,
            found: DataTypeId::Int32,
        }
    );
}
