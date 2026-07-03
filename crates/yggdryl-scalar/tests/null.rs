//! Integration tests for the `null` scalar — always null, holding no value.

use yggdryl_scalar::yggdryl_dtype::{DataError, RawDataType};
use yggdryl_scalar::{arrow_array, arrow_schema, Null, RawScalar};

#[test]
fn null_scalar_is_always_null() {
    let nothing = Null::new();
    assert!(nothing.is_null());
    assert_eq!(nothing.value(), None);
    assert_eq!(nothing.data_type().name(), "null");
    assert!(matches!(nothing.as_i64(), Err(DataError::NullValue)));
    assert!(matches!(nothing.as_str(None), Err(DataError::NullValue)));
    assert!(matches!(nothing.as_bytes(), Err(DataError::NullValue)));
}

#[test]
fn null_scalar_arrow_round_trips() {
    use arrow_array::Array;

    let nothing = Null::new();
    let arrow = nothing.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(arrow.data_type(), &arrow_schema::DataType::Null);
    assert_eq!(Null::from_arrow(arrow.as_ref()).unwrap(), nothing);

    // Wrong length and wrong type are refused.
    assert!(matches!(
        Null::from_arrow(&arrow_array::NullArray::new(2)),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    assert!(matches!(
        Null::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn null_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Null>();
}
