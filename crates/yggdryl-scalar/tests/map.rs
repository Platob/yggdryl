//! Integration tests for the `map` scalar — a sequence of key–value entries of
//! inner scalars.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError};
use yggdryl_scalar::{Int64Scalar, MapScalar, Scalar, UInt8Scalar};

type RankMap = MapScalar<dtype::UInt8Type, dtype::Int64Type, UInt8Scalar, Int64Scalar>;

#[test]
fn map_scalar_round_trips() {
    let scalar = RankMap::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(8), Int64Scalar::null()),
    ])
    .unwrap();
    assert!(!scalar.is_null());
    assert_eq!(scalar.value().map(<[_]>::len), Some(2));
    let arrow = scalar.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(RankMap::from_arrow(arrow.as_ref()).unwrap(), scalar);

    let missing = RankMap::null();
    assert!(missing.is_null());
    assert_eq!(
        RankMap::from_arrow(missing.to_arrow().as_ref()).unwrap(),
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
fn map_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RankMap>();
}
