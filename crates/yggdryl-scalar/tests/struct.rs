//! Tests for the recursive schema nodes: `StructType` / `StructField` / `AnyType` /
//! `AnyField`. (The `Any` / `Struct` values themselves are tested in `scalar.rs`.)

use yggdryl_scalar::{AnyField, AnyType, DataType, DataTypeId, Field, StructField, StructType};

#[test]
fn struct_type_holds_heterogeneous_child_fields() {
    let ty = StructType::new(vec![
        AnyField::int64("id"),
        AnyField::new("tag", DataTypeId::Utf8), // a DataTypeId redirects to the type
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
    let inner = AnyType::struct_type(StructType::new(vec![AnyField::int32("x")]));
    let schema = StructField::new(
        "record",
        vec![AnyField::int64("id"), AnyField::new("point", inner)],
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
    let field = AnyField::int32("a");
    let nullable = field.with_nullable(true);
    assert!(nullable.nullable());
    assert!(!field.nullable()); // original untouched

    let schema = StructField::new("s", vec![]);
    assert_eq!(schema.with_name("t".to_string()).name(), "t");
    assert_eq!(schema.name(), "s");
}
