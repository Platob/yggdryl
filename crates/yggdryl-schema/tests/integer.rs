//! Tests for the concrete integer type / field families.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use yggdryl_schema::{
    DataType, DataTypeId, Field, Int32Field, Int32Type, Int64Type, Metadata, NestedFields,
    PrimitiveField, PrimitiveType, UInt64Field, UInt8Type,
};

fn assert_primitive_type<T: PrimitiveType>(_t: &T) {}
fn assert_primitive_field<F: PrimitiveField>(_f: &F) {}

#[test]
fn integer_types_report_identity_and_category() {
    assert_eq!(Int32Type::new().type_name(), "int32");
    assert_eq!(Int32Type::new().type_id(), DataTypeId::Int32);
    assert_eq!(UInt8Type::new().type_name(), "uint8");
    assert_eq!(UInt8Type::new().type_id(), DataTypeId::UInt8);
    assert!(Int32Type::new().children_fields().is_empty());
    assert_primitive_type(&Int32Type::new());
    assert_primitive_type(&UInt8Type::new());
}

#[test]
fn integer_fields_wrap_their_type() {
    let field = Int32Field::new("count");
    assert_eq!(field.name(), "count");
    assert_eq!(field.dtype().type_id(), DataTypeId::Int32);
    assert!(field.metadata().is_none());
    assert!(field.children_fields().is_empty());
    assert_primitive_field(&field);
}

#[test]
fn integer_field_with_updates_are_non_mutating() {
    let field = UInt64Field::new("a");
    assert_eq!(field.with_name("b".to_string()).name(), "b");
    assert_eq!(field.name(), "a"); // original untouched

    let mut meta = Metadata::new();
    meta.insert(b"unit".to_vec(), b"bytes".to_vec());
    let with = field.with_metadata(meta.clone());
    assert_eq!(with.metadata(), Some(&meta));
    assert!(with.without_metadata().metadata().is_none());
}

#[test]
fn distinct_types_differ_but_a_clone_is_equal() {
    let a: Box<dyn DataType> = Box::new(Int32Type::new());
    let b: Box<dyn DataType> = Box::new(Int64Type::new());
    assert!(!a.dyn_eq(b.as_ref()));
    assert!(a.dyn_eq(a.clone_box().as_ref()));
}

#[test]
fn fields_of_different_types_are_not_equal() {
    let a: Box<dyn Field> = Box::new(Int32Field::new("x"));
    let b: Box<dyn Field> = Box::new(UInt64Field::new("x"));
    // Same name, different data type → not equal.
    assert!(!a.dyn_eq(b.as_ref()));
    // A clone hashes the same.
    assert_eq!(field_hash(a.as_ref()), field_hash(a.clone_box().as_ref()));
}

fn field_hash(field: &dyn Field) -> u64 {
    let mut hasher = DefaultHasher::new();
    field.dyn_hash(&mut hasher);
    hasher.finish()
}
