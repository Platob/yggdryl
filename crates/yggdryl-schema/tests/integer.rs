//! Tests for the concrete integer type / field families.

use yggdryl_schema::{
    DataType, DataTypeId, Field, Int256Field, Int256Type, Int32Field, Int32Type, Metadata,
    PrimitiveField, PrimitiveType, UInt64Field, UInt8Type, I256, U256,
};

fn assert_primitive_type<T, D: PrimitiveType<T>>(_d: &D) {}
fn assert_primitive_field<T, F: PrimitiveField<T>>(_f: &F) {}

#[test]
fn integer_types_report_identity_default_and_category() {
    assert_eq!(Int32Type::new().type_name(), "int32");
    assert_eq!(Int32Type::new().type_id(), DataTypeId::Int32);
    assert_eq!(Int32Type::new().default(), 0i32);
    assert_eq!(UInt8Type::new().type_id(), DataTypeId::UInt8);
    assert_eq!(UInt8Type::new().default(), 0u8);
    // The 256-bit types default over the custom native structs.
    assert_eq!(Int256Type::new().default(), I256::ZERO);
    assert_primitive_type(&Int32Type::new());
    assert_primitive_type(&Int256Type::new());
}

#[test]
fn integer_fields_wrap_their_type_and_default() {
    let field = Int32Field::new("count");
    assert_eq!(field.name(), "count");
    assert_eq!(field.dtype().type_id(), DataTypeId::Int32);
    assert!(!field.nullable()); // non-nullable by default
    assert_eq!(field.default(), Some(0i32)); // non-nullable → Some
    assert_eq!(field.with_nullable(true).default(), None); // nullable → None
    assert!(field.metadata().is_none());
    assert_primitive_field(&field);

    // The 256-bit field delegates its default to the custom native type.
    assert_eq!(Int256Field::new("big").default(), Some(I256::ZERO));
    assert_eq!(
        yggdryl_schema::UInt256Field::new("big").default(),
        Some(U256::ZERO)
    );
}

#[test]
fn integer_field_with_updates_are_non_mutating() {
    let field = UInt64Field::new("a");
    assert_eq!(field.with_name("b".to_string()).name(), "b");
    assert_eq!(field.name(), "a"); // original untouched

    // Nullability is a non-mutating flag.
    let nullable = field.with_nullable(true);
    assert!(nullable.nullable());
    assert!(!field.nullable()); // original untouched
    assert_eq!(nullable.name(), "a"); // other parts preserved

    let mut meta = Metadata::new();
    meta.insert(b"unit".to_vec(), b"bytes".to_vec());
    let with = field.with_metadata(meta.clone());
    assert_eq!(with.metadata(), Some(&meta));
    assert!(with.without_metadata().metadata().is_none());
}
