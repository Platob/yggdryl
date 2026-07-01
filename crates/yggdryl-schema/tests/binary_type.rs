//! Tests for [`BinaryType`] and the [`DataType`] hooks.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl_schema::{BinaryType, DataType, DataTypeId, PrimitiveType};

fn assert_primitive<T: PrimitiveType>(_t: &T) {}

#[test]
fn reports_its_identity() {
    let dt = BinaryType::new();
    assert_eq!(dt.type_id(), DataTypeId::Binary);
    assert_eq!(dt.type_name(), "binary");
}

#[test]
fn is_a_primitive_type() {
    assert_primitive(&BinaryType::new());
}

#[test]
fn boxed_data_type_clones_compares_and_hashes() {
    let a: Box<dyn DataType> = Box::new(BinaryType::new());
    let b = a.clone_box();
    // Equal through the dynamic hook...
    assert!(a.dyn_eq(b.as_ref()));
    // ...and equal hashes.
    assert_eq!(hash_of(a.as_ref()), hash_of(b.as_ref()));
}

fn hash_of(dt: &dyn DataType) -> u64 {
    let mut hasher = DefaultHasher::new();
    dt.type_id().hash(&mut hasher);
    dt.dyn_hash(&mut hasher);
    hasher.finish()
}
