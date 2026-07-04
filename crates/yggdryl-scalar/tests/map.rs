//! Integration tests for the `map` scalar — a sequence of key–value entries of
//! inner scalars.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, DataType};
use yggdryl_scalar::{Int64Scalar, MapScalar, Scalar, TypedMapScalar, UInt8Scalar};

type RankMap = TypedMapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar>;

#[test]
fn map_scalar_round_trips() {
    let scalar = RankMap::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(8), Int64Scalar::null()),
    ])
    .unwrap();
    assert!(!scalar.is_null());
    assert_eq!(scalar.value().map(<[_]>::len), Some(2));
    let arrow = scalar.to_arrow_scalar();
    assert_eq!(arrow.len(), 1);
    assert_eq!(RankMap::from_arrow(arrow.as_ref()).unwrap(), scalar);

    let missing = RankMap::null();
    assert!(missing.is_null());
    assert_eq!(
        RankMap::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );
    assert_eq!(RankMap::default(), RankMap::new(Vec::new()).unwrap());

    // A null key is refused: Arrow map keys are non-nullable.
    assert!(matches!(
        RankMap::new(vec![(UInt8Scalar::null(), Int64Scalar::new(1))]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn dynamic_map_scalar_round_trips() {
    // Erasing a typed map drops the key and value scalar types to the dynamic base,
    // over the same entries assembled into an Arrow struct array.
    let typed = RankMap::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(8), Int64Scalar::null()),
    ])
    .unwrap();
    let dynamic = typed.erase();
    assert!(!dynamic.is_null());
    assert_eq!(dynamic.len(), 2);
    assert!(!dynamic.is_empty());
    assert_eq!(dynamic.data_type().name(), "map");

    // The dynamic map round-trips through Arrow, and its Arrow form matches the
    // typed scalar's — erasing loses the static types, not the value.
    assert_eq!(
        MapScalar::from_arrow(dynamic.to_arrow_scalar().as_ref()).unwrap(),
        dynamic
    );
    let dynamic_arrow = dynamic.to_arrow_scalar();
    let typed_arrow = typed.to_arrow_scalar();
    assert_eq!(dynamic_arrow.as_ref(), typed_arrow.as_ref());

    // The null map erases to the dynamic null map.
    let missing = RankMap::null().erase();
    assert!(missing.is_null());
    assert_eq!(missing.len(), 0);
    assert_eq!(
        MapScalar::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );
}

#[test]
fn map_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RankMap>();
    assert_send_sync::<MapScalar>();
}
