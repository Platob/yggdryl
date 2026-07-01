//! Tests for the recursive struct model: `Struct` / `Any` values and
//! `StructType` / `StructField` / `AnyType` / `AnyField` schema nodes.

use yggdryl_schema::{
    Any, AnyField, AnyType, DataType, DataTypeId, Field, Struct, StructField, StructType,
};

#[test]
fn any_values_report_their_type() {
    assert_eq!(Any::default(), Any::Null);
    assert!(Any::Null.is_null());
    assert_eq!(Any::Int32(7).type_id(), DataTypeId::Int32);
    assert_eq!(Any::UInt8(255).type_id(), DataTypeId::UInt8);
}

#[test]
fn struct_value_is_an_array_of_any() {
    let row = Struct::new(vec![Any::Int32(1), Any::Null, Any::UInt8(2)]);
    assert_eq!(row.len(), 3);
    assert!(!row.is_empty());
    assert_eq!(row.get(0), Some(&Any::Int32(1)));
    assert_eq!(row.values()[1], Any::Null);
    // A struct can nest another struct value.
    let nested = Any::Struct(Struct::new(vec![Any::Int64(9)]));
    assert_eq!(nested.type_id(), DataTypeId::Struct);
}

#[test]
fn struct_type_holds_heterogeneous_child_fields() {
    let ty = StructType::new(vec![
        AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
        AnyField::new("tag", AnyType::primitive(DataTypeId::Utf8)),
    ]);
    assert_eq!(ty.type_id(), DataTypeId::Struct);
    assert_eq!(ty.type_name(), "struct");
    assert_eq!(ty.len(), 2);
    assert_eq!(ty.field_at(0).map(AnyField::name), Some("id"));
    assert_eq!(ty.field_by("tag").map(AnyField::name), Some("tag"));
    assert_eq!(
        ty.field_by("tag").map(|f| f.any_type().type_id()),
        Some(DataTypeId::Utf8)
    );
    assert!(ty.field_by("missing").is_none());
}

#[test]
fn struct_field_is_the_recursive_schema_node() {
    // A struct field whose children include a nested struct — full recursivity.
    let inner = AnyType::struct_type(StructType::new(vec![AnyField::new(
        "x",
        AnyType::primitive(DataTypeId::Int32),
    )]));
    let schema = StructField::new(
        "record",
        vec![
            AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
            AnyField::new("point", inner),
        ],
    );

    assert_eq!(schema.name(), "record");
    assert_eq!(schema.dtype().type_id(), DataTypeId::Struct);
    assert!(!schema.nullable());
    assert_eq!(schema.dtype().len(), 2);

    // Reach into the nested struct field.
    let point = schema.dtype().field_by("point").unwrap();
    assert_eq!(point.any_type().type_id(), DataTypeId::Struct);
    match point.any_type() {
        AnyType::Struct(inner) => {
            assert_eq!(inner.field_by("x").map(AnyField::name), Some("x"));
        }
        _ => panic!("expected a nested struct"),
    }
}

#[test]
fn field_updates_are_non_mutating() {
    let field = AnyField::new("a", AnyType::primitive(DataTypeId::Int32));
    let nullable = field.with_nullable(true);
    assert!(nullable.nullable());
    assert!(!field.nullable()); // original untouched

    let schema = StructField::new("s", vec![]);
    assert_eq!(schema.with_name("t".to_string()).name(), "t");
    assert_eq!(schema.name(), "s");
}
