//! Tests for [`Field`] and its functional updates.

use std::collections::HashSet;

use yggdryl_schema::{BinaryType, DataType, DataTypeId, Field};

fn binary() -> Box<dyn DataType> {
    Box::new(BinaryType::new())
}

#[test]
fn holds_a_name_data_type_and_no_metadata() {
    let field = Field::new("payload", binary());
    assert_eq!(field.name(), "payload");
    assert_eq!(field.dtype().type_id(), DataTypeId::Binary);
    assert!(field.metadata().is_none());
}

#[test]
fn with_updates_are_non_mutating() {
    let field = Field::new("a", binary());
    let renamed = field.with_name("b".to_string());
    assert_eq!(field.name(), "a"); // original untouched
    assert_eq!(renamed.name(), "b");
    assert_eq!(renamed.dtype().type_id(), DataTypeId::Binary);
}

#[test]
fn metadata_is_carried_and_cleared() {
    let mut meta = std::collections::BTreeMap::new();
    meta.insert(b"encoding".to_vec(), b"utf-8".to_vec());

    let with = Field::new("a", binary()).with_metadata(meta.clone());
    assert_eq!(with.metadata(), Some(&meta));

    let without = with.without_metadata();
    assert!(without.metadata().is_none());
}

#[test]
fn clone_equals_and_hashes_as_the_original() {
    let field = Field::new("a", binary());
    let twin = field.clone();
    assert_eq!(field, twin);

    // Equal fields with equal hashes collapse in a set.
    let set: HashSet<Field> = [field, twin].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn differing_names_are_not_equal() {
    assert_ne!(Field::new("a", binary()), Field::new("b", binary()));
}
