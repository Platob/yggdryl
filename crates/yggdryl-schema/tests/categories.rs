//! Tests the [`LogicalType`] / [`NestedType`] signatures and default lookups via
//! minimal dummy implementations (no such concrete type ships yet).

use std::hash::Hasher;

use yggdryl_schema::{BinaryType, DataType, DataTypeId, Field, LogicalType, NestedType};

/// A stand-in nested type holding child fields.
#[derive(Debug)]
struct Structish(Vec<Field>);

impl DataType for Structish {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
    fn type_name(&self) -> &str {
        "struct"
    }
    fn clone_box(&self) -> Box<dyn DataType> {
        Box::new(Structish(self.0.clone()))
    }
    fn dyn_eq(&self, other: &dyn DataType) -> bool {
        other.type_id() == self.type_id()
    }
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        use std::hash::Hash;
        self.type_id().hash(&mut state);
    }
}

impl NestedType for Structish {
    fn children_fields(&self) -> &[Field] {
        &self.0
    }
}

#[test]
fn nested_type_looks_up_children_by_index_and_name() {
    let ty = Structish(vec![
        Field::new("id", Box::new(BinaryType::new())),
        Field::new("body", Box::new(BinaryType::new())),
    ]);

    assert_eq!(ty.children_fields().len(), 2);
    assert_eq!(ty.child_field_at(1).map(Field::name), Some("body"));
    assert!(ty.child_field_at(2).is_none());
    assert_eq!(ty.child_field_by("id").map(Field::name), Some("id"));
    assert!(ty.child_field_by("missing").is_none());
}

/// A stand-in logical type wrapping a physical storage type.
#[derive(Debug)]
struct Logicalish(BinaryType);

impl DataType for Logicalish {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Utf8
    }
    fn type_name(&self) -> &str {
        "utf8"
    }
    fn clone_box(&self) -> Box<dyn DataType> {
        Box::new(Logicalish(self.0))
    }
    fn dyn_eq(&self, other: &dyn DataType) -> bool {
        other.type_id() == self.type_id()
    }
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        use std::hash::Hash;
        self.type_id().hash(&mut state);
    }
}

impl LogicalType for Logicalish {
    fn inner_type(&self) -> &dyn DataType {
        &self.0
    }
}

#[test]
fn logical_type_exposes_its_inner_type() {
    let ty = Logicalish(BinaryType::new());
    assert_eq!(ty.type_name(), "utf8");
    assert_eq!(ty.inner_type().type_id(), DataTypeId::Binary);
}
