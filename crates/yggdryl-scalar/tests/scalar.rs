//! Tests for the scalar values: `Any`, `Struct`, and the generic `Scalar` trait
//! implemented by natives, `Any` and `Struct`.

use std::collections::HashSet;

use yggdryl_scalar::{Any, DataTypeId, Scalar, Struct, U256};

#[test]
fn natives_are_scalars() {
    assert_eq!(7i32.type_id(), DataTypeId::Int32);
    assert_eq!(7i32.to_any(), Any::Int32(7));
    assert!(!7i32.is_null());

    assert_eq!(U256::from(5u8).type_id(), DataTypeId::UInt256);
    assert_eq!(U256::from(5u8).to_any(), Any::UInt256(U256::from(5u8)));
}

#[test]
fn any_reports_type_and_reads_atoms() {
    let v = Any::from(255u8);
    assert_eq!(v, Any::UInt8(255));
    assert_eq!(v.type_id(), DataTypeId::UInt8);
    assert_eq!(v.as_u8(), Some(255));
    assert_eq!(v.as_i8(), None); // wrong type → None
    assert!(!v.is_null());
    assert!(!v.is_struct());

    assert!(Any::Null.is_null());
    assert_eq!(Any::default(), Any::Null);
    assert_eq!(Any::Null.to_any(), Any::Null); // Any is itself a Scalar
}

#[test]
fn struct_is_an_array_of_any_built_from_scalars() {
    // From native scalars — each promotes to an `Any` child.
    let row = Struct::from_scalars([1i32, 2i32]);
    assert_eq!(row.len(), 2);
    assert!(!row.is_empty());
    assert_eq!(row.get(0), Some(&Any::Int32(1)));
    assert_eq!(row.values()[1], Any::Int32(2));
    assert_eq!(row.type_id(), DataTypeId::Struct);

    // Explicit `Any` children work too.
    let mixed = Struct::new(vec![Any::Int64(9), Any::Null, Any::from(3u8)]);
    assert_eq!(mixed.len(), 3);
    assert_eq!(mixed.get(2), Some(&Any::UInt8(3)));
}

#[test]
fn structs_nest_recursively() {
    let inner = Struct::from_scalars([1i32, 2i32]);
    // A struct is itself a scalar, so it nests via `to_any`.
    let outer = Struct::new(vec![Any::Int32(0), inner.to_any()]);
    assert_eq!(outer.len(), 2);
    let child = outer.get(1).unwrap();
    assert!(child.is_struct());
    assert_eq!(child.as_struct().unwrap().len(), 2);
    assert_eq!(child.as_struct().unwrap().get(0), Some(&Any::Int32(1)));
    // `from_scalars` accepts nested structs directly.
    let from_structs = Struct::from_scalars([inner.clone(), inner]);
    assert!(from_structs.get(0).unwrap().is_struct());
}

#[test]
fn scalars_are_hashable_and_eq() {
    assert_eq!(Any::from(1i32), Any::Int32(1));

    let mut set = HashSet::new();
    set.insert(Struct::from_scalars([1i32, 2i32]));
    set.insert(Struct::from_scalars([1i32, 2i32]));
    assert_eq!(set.len(), 1);
}
