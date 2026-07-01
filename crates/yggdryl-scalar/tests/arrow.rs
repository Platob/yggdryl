//! Tests for the dynamic / nested Arrow round-trip: `AnyType` / `AnyField` and the
//! recursive `StructType` / `StructField`. The primitive Arrow round-trip is tested in
//! the `yggdryl-schema` crate.

use yggdryl_scalar::{
    AnyField, AnyType, ArrowArray, ArrowError, ArrowSchema, ArrowSchemaExt, DataType, DataTypeId,
    Field, Metadata, StructField, StructType,
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
    assert!(node.metadata().contains_key(EXT_KEY));
    assert_eq!(
        node.metadata().get(b"unit".as_slice()).map(Vec::as_slice),
        Some(b"wei".as_slice())
    );
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
    assert_eq!(node.children()[2].format(), "+s");
    assert_eq!(node.children()[2].children().len(), 2);

    assert_eq!(StructField::from_arrow(&node).unwrap(), schema);
}

#[test]
fn from_arrow_rejects_unmodelled_and_malformed_nodes() {
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

    let scalar = AnyType::primitive(DataTypeId::Int32).to_arrow();
    assert_eq!(
        StructField::from_arrow(&scalar),
        Err(ArrowError::NotAStruct("i".to_string()))
    );
}

#[test]
fn arrow_schema_converts_to_and_from_struct_field() {
    let schema = StructField::new(
        "record",
        vec![
            AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
            AnyField::new("tag", AnyType::primitive(DataTypeId::Utf8)),
        ],
    )
    .with_nullable(true);

    let arrow = ArrowSchema::from_struct_field(&schema);
    assert_eq!(arrow.format(), "+s");
    assert_eq!(arrow, schema.to_arrow());
    assert_eq!(arrow.to_struct_field().unwrap(), schema);

    // A non-struct node cannot become a StructField.
    let scalar = AnyType::primitive(DataTypeId::Int32).to_arrow();
    assert_eq!(
        scalar.to_struct_field().unwrap_err(),
        ArrowError::NotAStruct("i".to_string())
    );
}

#[test]
fn field_from_arrow_array_takes_nullability_from_null_count() {
    let schema = AnyField::new("id", AnyType::primitive(DataTypeId::Int64)).to_arrow();
    assert!(!schema.nullable());

    let none = AnyField::from_arrow_array(&schema, &ArrowArray::from_parts(5, 0, vec![])).unwrap();
    assert!(!none.nullable());
    assert_eq!(none.name(), "id");
    assert_eq!(none.any_type().type_id(), DataTypeId::Int64);

    let some = AnyField::from_arrow_array(&schema, &ArrowArray::from_parts(5, 1, vec![])).unwrap();
    assert!(some.nullable());

    let unknown =
        AnyField::from_arrow_array(&schema, &ArrowArray::from_parts(5, -1, vec![])).unwrap();
    assert!(unknown.nullable());
}

#[test]
fn struct_from_arrow_array_threads_nullability_into_children() {
    let schema = StructField::new(
        "record",
        vec![
            AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
            AnyField::new("name", AnyType::primitive(DataTypeId::Utf8)),
        ],
    )
    .to_arrow();

    let array = ArrowArray::from_parts(
        4,
        0,
        vec![
            ArrowArray::from_parts(4, 1, vec![]),
            ArrowArray::from_parts(4, 0, vec![]),
        ],
    );

    let field = StructField::from_arrow_array(&schema, &array).unwrap();
    assert!(!field.nullable());
    let children = field.dtype().fields();
    assert!(children[0].nullable());
    assert!(!children[1].nullable());

    let short = ArrowArray::from_parts(4, 0, vec![ArrowArray::from_parts(4, 0, vec![])]);
    assert_eq!(
        StructField::from_arrow_array(&schema, &short).unwrap_err(),
        ArrowError::ChildCountMismatch {
            schema: 2,
            array: 1,
        }
    );
}
