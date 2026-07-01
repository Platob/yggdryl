//! Tests for the scalar value layer: primitive, dynamic and nested scalars built from
//! native values and collections.

use std::collections::HashSet;

use yggdryl_scalar::{Any, AnyValue, DataType, DataTypeId, Int32, Scalar, Struct, UInt256, U256};

#[test]
fn primitive_scalar_builds_from_native() {
    let s = Int32::from(7);
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
    let s = UInt256::from(U256::from(5u8));
    assert_eq!(*s.value(), U256::from(5u8));
    assert_eq!(s.dtype().type_id(), DataTypeId::UInt256);
}

#[test]
fn any_scalar_builds_from_native() {
    let s = Any::from(255u8);
    assert_eq!(*s.value(), AnyValue::UInt8(255));
    assert_eq!(s.field().any_type().type_id(), DataTypeId::UInt8);
    assert_eq!(s.name(), "");
}

#[test]
fn typed_scalar_converts_to_any_scalar() {
    let any: Any = Int32::from(3).with_name("x".to_string()).into();
    assert_eq!(*any.value(), AnyValue::Int32(3));
    assert_eq!(any.name(), "x");
    assert_eq!(any.field().any_type().type_id(), DataTypeId::Int32);
}

#[test]
fn struct_scalar_from_collection_of_scalars() {
    let row = Struct::new(
        "point",
        vec![
            Int32::from(1).with_name("x".to_string()).into(),
            Int32::from(2).with_name("y".to_string()).into(),
        ],
    );
    assert_eq!(row.name(), "point");
    assert_eq!(row.dtype().type_id(), DataTypeId::Struct);
    assert_eq!(row.len(), 2);

    let scalars = row.scalars();
    assert_eq!(scalars.len(), 2);
    assert_eq!(scalars[0].name(), "x");
    assert_eq!(*scalars[1].value(), AnyValue::Int32(2));

    // The `From<Vec<Any>>` builder gives an unnamed struct scalar.
    let unnamed = Struct::from(vec![Any::from(1i64)]);
    assert_eq!(unnamed.name(), "");
    assert_eq!(unnamed.len(), 1);
}

#[test]
fn struct_scalars_nest_recursively() {
    let inner = Struct::new(
        "point",
        vec![Int32::from(1).with_name("x".to_string()).into()],
    );
    let outer = Struct::new(
        "record",
        vec![
            Any::from(10i64).with_name("id".to_string()),
            inner.into(), // Struct -> Any, nested as a child
        ],
    );
    assert_eq!(outer.len(), 2);

    let children = outer.scalars();
    assert_eq!(children[0].name(), "id");
    assert_eq!(children[1].field().any_type().type_id(), DataTypeId::Struct);
    assert!(matches!(children[1].value(), AnyValue::Struct(_)));
}

#[test]
fn any_scalar_atomic_accessors_read_the_native_value() {
    let s = Any::from(42i64);
    assert_eq!(s.as_i64(), Some(42));
    assert_eq!(s.as_i32(), None); // wrong type → None
    assert_eq!(s.as_u64(), None);
    assert!(!s.is_null());
    assert!(!s.is_struct());
}

#[test]
fn struct_scalar_navigates_children_atomically() {
    let row = Struct::new(
        "point",
        vec![
            Int32::from(1).with_name("x".to_string()).into(),
            Int32::from(2).with_name("y".to_string()).into(),
        ],
    );

    // Positional and named atomic access.
    assert_eq!(row.scalar_at(0).unwrap().as_i32(), Some(1));
    assert_eq!(row.scalar_by("y").unwrap().as_i32(), Some(2));
    assert_eq!(row.scalar_by("y").unwrap().name(), "y");
    assert!(row.scalar_at(2).is_none()); // out of range
    assert!(row.scalar_by("z").is_none()); // unknown name

    // A struct-valued scalar reports as a struct and yields its struct value.
    let nested: Any = row.into();
    assert!(nested.is_struct());
    assert_eq!(nested.as_struct().unwrap().len(), 2);
    assert_eq!(nested.as_i32(), None);
}

#[test]
fn scalars_are_hashable_and_eq() {
    let a = Int32::from(1).with_name("n".to_string());
    let b = Int32::from(1).with_name("n".to_string());
    assert_eq!(a, b);

    let mut set = HashSet::new();
    set.insert(Any::from(1i32));
    set.insert(Any::from(1i32));
    assert_eq!(set.len(), 1);
}
