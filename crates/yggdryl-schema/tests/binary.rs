//! Tests for the concrete [`BinaryType`] / [`BinaryField`] pair.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use yggdryl_schema::{
    BinaryField, BinaryType, DataType, DataTypeId, Field, Metadata, NestedFields, PrimitiveField,
    PrimitiveType,
};

fn assert_primitive_type<T: PrimitiveType>(_t: &T) {}
fn assert_primitive_field<F: PrimitiveField>(_f: &F) {}

#[test]
fn binary_type_identity_and_category() {
    let dt = BinaryType::new();
    assert_eq!(dt.type_id(), DataTypeId::Binary);
    assert_eq!(dt.type_name(), "binary");
    assert!(dt.children_fields().is_empty());
    assert_primitive_type(&dt);
}

#[test]
fn binary_field_identity_and_category() {
    let field = BinaryField::new("payload");
    assert_eq!(field.name(), "payload");
    assert_eq!(field.dtype().type_id(), DataTypeId::Binary);
    assert!(field.metadata().is_none());
    assert!(field.children_fields().is_empty());
    assert_primitive_field(&field);
}

#[test]
fn binary_field_with_updates_are_non_mutating() {
    let field = BinaryField::new("a");
    let renamed = field.with_name("b".to_string());
    assert_eq!(field.name(), "a"); // original untouched
    assert_eq!(renamed.name(), "b");

    let mut meta = Metadata::new();
    meta.insert(b"k".to_vec(), b"v".to_vec());
    let with = field.with_metadata(meta.clone());
    assert_eq!(with.metadata(), Some(&meta));
    assert!(with.without_metadata().metadata().is_none());
}

#[test]
fn fields_compare_and_hash_by_name_dtype_metadata() {
    let a: Box<dyn Field> = Box::new(BinaryField::new("x"));
    let b: Box<dyn Field> = Box::new(BinaryField::new("x"));
    let c: Box<dyn Field> = Box::new(BinaryField::new("y"));

    assert!(a.dyn_eq(b.as_ref()));
    assert!(!a.dyn_eq(c.as_ref()));
    assert_eq!(field_hash(a.as_ref()), field_hash(b.as_ref()));

    // clone_box yields an equal field.
    assert!(a.dyn_eq(a.clone_box().as_ref()));
}

fn field_hash(field: &dyn Field) -> u64 {
    let mut hasher = DefaultHasher::new();
    field.dyn_hash(&mut hasher);
    hasher.finish()
}
