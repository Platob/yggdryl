//! Tests the shared [`NestedFields`] lookups (via a dummy nested type) and the
//! [`LogicalType`] `inner_type` accessor (via a dummy logical type). No concrete
//! nested/logical type ships yet.

use std::hash::Hasher;

use yggdryl_schema::{
    BinaryField, BinaryType, DataType, DataTypeId, Field, LogicalType, NestedFields, NestedType,
    SchemaError,
};

/// A stand-in nested type holding child fields.
#[derive(Debug)]
struct Structish(Vec<Box<dyn Field>>);

impl NestedFields for Structish {
    fn children_fields(&self) -> &[Box<dyn Field>] {
        &self.0
    }
}

impl DataType for Structish {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
    fn type_name(&self) -> &str {
        "struct"
    }
    fn clone_box(&self) -> Box<dyn DataType> {
        Box::new(Structish(self.0.iter().map(|f| f.clone_box()).collect()))
    }
}

impl NestedType for Structish {}

fn structish() -> Structish {
    Structish(vec![
        Box::new(BinaryField::new("Id")),
        Box::new(BinaryField::new("Body")),
    ])
}

#[test]
fn lookup_by_index_and_by_name() {
    let ty = structish();
    assert_eq!(ty.children_fields().len(), 2);
    assert_eq!(ty.child_field_at(1).map(|f| f.name()), Some("Body"));
    assert!(ty.child_field_at(2).is_none());
    // Exact name match.
    assert_eq!(ty.child_field_by("Id", true).map(|f| f.name()), Some("Id"));
}

#[test]
fn name_lookup_is_case_insensitive_by_default() {
    let ty = structish();
    // Case-insensitive fallback finds it...
    assert_eq!(
        ty.child_field_by("body", false).map(|f| f.name()),
        Some("Body")
    );
    // ...but strict matching does not.
    assert!(ty.child_field_by("body", true).is_none());
}

#[test]
fn child_field_combines_index_and_name() {
    let ty = structish();

    // Both: the index is used and its name matches.
    assert_eq!(
        ty.child_field(Some(0), Some("Id"), true)
            .unwrap()
            .map(|f| f.name()),
        Some("Id")
    );
    // Both, but the index's name mismatches → falls back to a name search.
    assert_eq!(
        ty.child_field(Some(0), Some("Body"), true)
            .unwrap()
            .map(|f| f.name()),
        Some("Body")
    );
    // Index only.
    assert_eq!(
        ty.child_field(Some(1), None, false)
            .unwrap()
            .map(|f| f.name()),
        Some("Body")
    );
    // Name only (case-insensitive).
    assert_eq!(
        ty.child_field(None, Some("id"), false)
            .unwrap()
            .map(|f| f.name()),
        Some("Id")
    );
    // Neither selector → error.
    assert_eq!(
        ty.child_field(None, None, false).unwrap_err(),
        SchemaError::NoChildSelector
    );
}

/// A stand-in logical type wrapping a physical storage type.
#[derive(Debug)]
struct Dictish(BinaryType);

impl NestedFields for Dictish {}

impl DataType for Dictish {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Utf8
    }
    fn type_name(&self) -> &str {
        "dictionary"
    }
    fn clone_box(&self) -> Box<dyn DataType> {
        Box::new(Dictish(self.0))
    }
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        use std::hash::Hash;
        self.type_id().hash(&mut state);
    }
}

impl LogicalType for Dictish {
    fn inner_type(&self) -> &dyn DataType {
        &self.0
    }
}

#[test]
fn logical_type_exposes_its_inner_type() {
    let ty = Dictish(BinaryType::new());
    assert_eq!(ty.type_name(), "dictionary");
    assert_eq!(ty.inner_type().type_id(), DataTypeId::Binary);
    // A logical leaf still has no child fields here.
    assert!(ty.children_fields().is_empty());
}
