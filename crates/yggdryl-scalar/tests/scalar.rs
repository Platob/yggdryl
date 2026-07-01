//! Tests for the scalar value layer: primitive, dynamic and nested scalars built from
//! native values and collections.

use std::collections::HashSet;

use yggdryl_scalar::{AnyScalar, Int32Scalar, Scalar, StructScalar, UInt256Scalar};
use yggdryl_schema::{Any, DataType, DataTypeId, U256};

#[test]
fn primitive_scalar_builds_from_native() {
    let s = Int32Scalar::from(7);
    assert_eq!(*s.value(), 7);
    assert_eq!(s.name(), ""); // unnamed until `with_name`
    assert_eq!(s.dtype().type_id(), DataTypeId::Int32);

    // `with_*` updates are non-mutating.
    let named = s.with_name("count".to_string());
    assert_eq!(named.name(), "count");
    assert_eq!(s.name(), "");

    let updated = named.with_value(9);
    assert_eq!(*updated.value(), 9);
    assert_eq!(updated.name(), "count");
    assert_eq!(*named.value(), 7);
}

#[test]
fn wide_scalar_builds_from_native() {
    let s = UInt256Scalar::from(U256::from(5u8));
    assert_eq!(*s.value(), U256::from(5u8));
    assert_eq!(s.dtype().type_id(), DataTypeId::UInt256);
}

#[test]
fn any_scalar_builds_from_native() {
    let s = AnyScalar::from(255u8);
    assert_eq!(*s.value(), Any::UInt8(255));
    assert_eq!(s.field().any_type().type_id(), DataTypeId::UInt8);
    assert_eq!(s.name(), "");
}

#[test]
fn typed_scalar_converts_to_any_scalar() {
    let any: AnyScalar = Int32Scalar::from(3).with_name("x".to_string()).into();
    assert_eq!(*any.value(), Any::Int32(3));
    assert_eq!(any.name(), "x");
    assert_eq!(any.field().any_type().type_id(), DataTypeId::Int32);
}

#[test]
fn struct_scalar_from_collection_of_scalars() {
    let row = StructScalar::new(
        "point",
        vec![
            Int32Scalar::from(1).with_name("x".to_string()).into(),
            Int32Scalar::from(2).with_name("y".to_string()).into(),
        ],
    );
    assert_eq!(row.name(), "point");
    assert_eq!(row.dtype().type_id(), DataTypeId::Struct);
    assert_eq!(row.value().len(), 2);

    let scalars = row.scalars();
    assert_eq!(scalars.len(), 2);
    assert_eq!(scalars[0].name(), "x");
    assert_eq!(*scalars[1].value(), Any::Int32(2));

    // The `From<Vec<AnyScalar>>` builder gives an unnamed struct scalar.
    let unnamed = StructScalar::from(vec![AnyScalar::from(1i64)]);
    assert_eq!(unnamed.name(), "");
    assert_eq!(unnamed.value().len(), 1);
}

#[test]
fn struct_scalars_nest_recursively() {
    let inner = StructScalar::new(
        "point",
        vec![Int32Scalar::from(1).with_name("x".to_string()).into()],
    );
    let outer = StructScalar::new(
        "record",
        vec![
            AnyScalar::from(10i64).with_name("id".to_string()),
            inner.into(), // StructScalar -> AnyScalar, nested as a child
        ],
    );
    assert_eq!(outer.value().len(), 2);

    let children = outer.scalars();
    assert_eq!(children[0].name(), "id");
    assert_eq!(children[1].field().any_type().type_id(), DataTypeId::Struct);
    assert!(matches!(children[1].value(), Any::Struct(_)));
}

#[test]
fn scalars_are_hashable_and_eq() {
    let a = Int32Scalar::from(1).with_name("n".to_string());
    let b = Int32Scalar::from(1).with_name("n".to_string());
    assert_eq!(a, b);

    let mut set = HashSet::new();
    set.insert(AnyScalar::from(1i32));
    set.insert(AnyScalar::from(1i32));
    assert_eq!(set.len(), 1);
}
