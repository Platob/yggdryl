//! Behavioural tests for the `yggdryl-scalar` primitive scalars — the value + null
//! surface, the byte codec, value semantics, and the guided error paths.

use arrow_schema::DataType as ArrowDataType;
use yggdryl_dtype::DataType;
use yggdryl_scalar::{
    BooleanScalar, F64Scalar, I64Scalar, Scalar, ScalarError, TypedScalar, U64Scalar,
};

#[test]
fn present_and_null_values() {
    let present = I64Scalar::new(7);
    assert_eq!(present.value(), Some(7));
    assert!(!present.is_null());
    assert_eq!(present.arrow_data_type(), ArrowDataType::Int64);
    assert_eq!(TypedScalar::data_type(&present).name(), "int64");

    let null = I64Scalar::null();
    assert_eq!(null.value(), None);
    assert!(null.is_null());
    // A null still knows its data type.
    assert_eq!(null.arrow_data_type(), ArrowDataType::Int64);
}

#[test]
fn byte_round_trip_present_and_null() {
    let present = U64Scalar::new(u64::MAX);
    let bytes = present.serialize_bytes();
    assert_eq!(bytes[0], 1); // present flag
    assert_eq!(U64Scalar::deserialize_bytes(&bytes).unwrap(), present);

    let null = U64Scalar::null();
    assert_eq!(null.serialize_bytes(), vec![0]);
    assert_eq!(U64Scalar::deserialize_bytes(&[0]).unwrap(), null);
}

#[test]
fn deserialize_errors_are_guided() {
    assert_eq!(
        I64Scalar::deserialize_bytes(&[]).unwrap_err(),
        ScalarError::EmptyPayload
    );
    // Flag neither 0 nor 1.
    assert_eq!(
        I64Scalar::deserialize_bytes(&[2]).unwrap_err(),
        ScalarError::InvalidNullFlag { flag: 2 }
    );
    // Null flag with stray value bytes.
    assert_eq!(
        I64Scalar::deserialize_bytes(&[0, 9]).unwrap_err(),
        ScalarError::NullWithValue { len: 1 }
    );
    // Present flag but a wrong-width value payload → underlying dtype error.
    let err = I64Scalar::deserialize_bytes(&[1, 0, 0]).unwrap_err();
    assert!(matches!(err, ScalarError::Dtype(_)));
    assert!(err.to_string().contains("8-byte"));
}

#[test]
fn float_value_semantics_are_bitwise() {
    // 0.0 and -0.0 are distinct (different bits); a present value never equals a null.
    assert_ne!(F64Scalar::new(0.0), F64Scalar::new(-0.0));
    assert_ne!(F64Scalar::new(1.0), F64Scalar::null());

    // Two NaNs with the same bit pattern are equal (byte-based equality).
    assert_eq!(F64Scalar::new(f64::NAN), F64Scalar::new(f64::NAN));
}

#[test]
fn value_semantics_as_set_keys() {
    use std::collections::HashSet;

    assert_eq!(I64Scalar::new(5), I64Scalar::new(5));
    assert_ne!(I64Scalar::new(5), I64Scalar::new(6));
    assert_eq!(I64Scalar::null(), I64Scalar::null());

    let mut set = HashSet::new();
    set.insert(I64Scalar::new(5));
    set.insert(I64Scalar::new(5));
    set.insert(I64Scalar::null());
    assert_eq!(set.len(), 2);
}

#[test]
fn boolean_scalar() {
    assert_eq!(BooleanScalar::new(true).value(), Some(true));
    assert_eq!(BooleanScalar::new(false).serialize_bytes(), vec![1, 0]);
    assert_eq!(BooleanScalar::new(true).serialize_bytes(), vec![1, 1]);
    assert_eq!(
        BooleanScalar::deserialize_bytes(&[1, 1]).unwrap(),
        BooleanScalar::new(true)
    );
    assert_eq!(BooleanScalar::null().serialize_bytes(), vec![0]);
}
