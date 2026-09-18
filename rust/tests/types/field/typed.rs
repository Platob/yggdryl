//! A field is an enum over its leaves, and each leaf carries its own datatype.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl::types::{DataTypeValue, FieldValue, Int64Field, Int64Type, StringField, StringType};
use yggdryl::{DataType, Field};

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn a_field_takes_the_leaf_its_datatype_names() {
    // The variant is not a caller's claim: it follows the datatype, and the
    // leaf then holds that datatype in its own type rather than a `DataType`
    // that something else has to agree with.
    let field = Field::new("id", DataType::Int64, false);
    assert!(matches!(field, Field::Int64(_)));
    assert_eq!(field.dtype(), &DataType::Int64);

    let leaf = Int64Field::from_field(&field).expect("the field is the Int64 leaf");
    assert_eq!(leaf.typed_dtype_ref(), &Int64Type);
    assert_eq!(leaf.name(), "id");

    // A leaf of another family narrows to nothing, which is the whole of what
    // the old marker proved.
    assert!(StringField::from_field(&field).is_none());
}

#[test]
fn a_leaf_and_the_field_it_widens_to_are_the_same_field() {
    let leaf = Int64Field::new("id", Int64Type, true);
    let field = Field::from(leaf.clone());

    assert_eq!(field.name(), leaf.name());
    assert_eq!(field.dtype(), leaf.dtype());
    assert_eq!(field.is_nullable(), leaf.is_nullable());

    // Round-tripping through the root changes nothing, and the leaf reads the
    // same datatype in either spelling.
    let narrowed = Int64Field::from_field(&field).expect("the leaf is still the Int64 one");
    assert_eq!(narrowed.typed_dtype_ref(), leaf.typed_dtype_ref());
    assert_eq!(stable_hash(&field), stable_hash(&Field::from(leaf)));
}

#[test]
fn replacing_the_datatype_moves_the_field_to_the_matching_leaf() {
    let mut field = Field::new("value", DataType::Int64, false);
    assert!(matches!(field, Field::Int64(_)));

    field
        .set_dtype(DataType::utf8())
        .expect("utf8 is a valid datatype");
    assert!(matches!(field, Field::String(_)));
    assert_eq!(field.dtype(), &DataType::utf8());
    assert_eq!(field.name(), "value", "the name survives the move");

    // The leaf follows the datatype, so the two can never disagree.
    let leaf = StringField::from_field(&field).expect("the field is the String leaf now");
    assert_eq!(
        leaf.typed_dtype_ref(),
        &StringType::from_dtype(&DataType::utf8()).expect("utf8 is a string datatype")
    );
}

#[test]
fn a_datatype_payload_reads_back_the_datatype_it_came_from() {
    // Every payload is a DataTypeValue, and widening then narrowing is the
    // identity - that is what lets a leaf store the payload and nothing else.
    for dtype in [DataType::Int64, DataType::utf8(), DataType::Boolean] {
        let field = Field::new("column", dtype.clone(), false);
        assert_eq!(field.dtype(), &dtype);
        assert_eq!(field.id(), dtype.id());
    }
}

/// Asserts that a datatype's payload reads back the datatype it came from.
///
/// The check the old compile-time markers used to make, on the type that
/// replaced them: a payload narrows out of the datatype it belongs to, refuses
/// every other one, and widens back to exactly what it came from.
pub fn assert_typed_marker<D: DataTypeValue>(dtype: DataType) {
    let payload =
        D::from_dtype(&dtype).unwrap_or_else(|| panic!("{dtype} should narrow to {}", D::FAMILY));
    assert_eq!(
        payload.clone().into_dtype(),
        dtype,
        "{} should widen back to the datatype it came from",
        D::FAMILY
    );
    assert_eq!(payload.id(), dtype.id());

    // A field of that datatype carries the payload and nothing else.
    let field = Field::new("column", dtype.clone(), false);
    assert_eq!(field.dtype(), &dtype);
}
