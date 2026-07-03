//! Tests for the `DataTypeId` classifier over every data type in the model.

use std::collections::HashSet;
use yggdryl_dtype::DataTypeId;

#[test]
fn every_id_has_a_unique_nonempty_name() {
    let mut names = HashSet::new();
    for id in DataTypeId::ALL {
        let name = id.name();
        assert!(!name.is_empty(), "{id:?} has an empty name");
        assert!(names.insert(name), "duplicate name {name:?}");
    }
    assert_eq!(names.len(), DataTypeId::ALL.len());
}

#[test]
fn all_contains_no_duplicates() {
    let unique: HashSet<_> = DataTypeId::ALL.iter().collect();
    assert_eq!(unique.len(), DataTypeId::ALL.len());
}

#[test]
fn primitive_and_nested_are_disjoint() {
    for id in DataTypeId::ALL {
        assert!(
            !(id.is_primitive() && id.is_nested()),
            "{id:?} is both primitive and nested"
        );
    }
}

#[test]
fn primitives_are_the_fixed_width_numerics_and_boolean() {
    let primitives: Vec<_> = DataTypeId::ALL
        .iter()
        .filter(|id| id.is_primitive())
        .collect();
    // boolean + 8 integers + 3 floats = 12.
    assert_eq!(primitives.len(), 12);
    assert!(DataTypeId::Int64.is_primitive());
    assert!(DataTypeId::Float64.is_primitive());
    assert!(!DataTypeId::Utf8.is_primitive());
    assert!(!DataTypeId::Timestamp.is_primitive()); // logical, not primitive
}

#[test]
fn nested_types_have_children() {
    for id in [
        DataTypeId::List,
        DataTypeId::LargeList,
        DataTypeId::FixedSizeList,
        DataTypeId::Struct,
        DataTypeId::Union,
        DataTypeId::Map,
    ] {
        assert!(id.is_nested(), "{id:?} should be nested");
    }
    assert!(!DataTypeId::Int64.is_nested());
}

#[test]
fn arrow_formats_are_the_known_parameterless_strings() {
    assert_eq!(DataTypeId::Null.arrow_format(), Some("n"));
    assert_eq!(DataTypeId::Int32.arrow_format(), Some("i"));
    assert_eq!(DataTypeId::Int64.arrow_format(), Some("l"));
    assert_eq!(DataTypeId::UInt64.arrow_format(), Some("L")); // capitalised = unsigned
    assert_eq!(DataTypeId::Float64.arrow_format(), Some("g"));
    assert_eq!(DataTypeId::Utf8.arrow_format(), Some("u"));
    assert_eq!(DataTypeId::Struct.arrow_format(), Some("+s"));

    // Parameterized / logical types have no id-level format.
    for id in [
        DataTypeId::Decimal128,
        DataTypeId::Timestamp,
        DataTypeId::FixedSizeBinary,
        DataTypeId::Union,
    ] {
        assert_eq!(
            id.arrow_format(),
            None,
            "{id:?} should have no static format"
        );
    }
}
