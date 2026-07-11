//! Behavioural tests for the `yggdryl-scalar` primitive scalars — the (always-present)
//! value surface, the byte codec, value semantics, and the guided error paths.

use arrow_schema::DataType as ArrowDataType;
use yggdryl_dtype::DataType;
use yggdryl_scalar::{
    BooleanScalar, F64Scalar, I64Scalar, Scalar, ScalarError, TypedScalar, U64Scalar,
};

#[test]
fn value_and_data_type() {
    let present = I64Scalar::new(7);
    assert_eq!(present.value(), 7); // always present — a plain value, not an Option
    assert_eq!(present.arrow_data_type(), ArrowDataType::Int64);
    assert_eq!(TypedScalar::data_type(&present).name(), "int64");
}

#[test]
fn byte_round_trip() {
    // A scalar serialises to just its value's little-endian bytes (no null flag).
    let present = U64Scalar::new(u64::MAX);
    let bytes = present.serialize_bytes();
    assert_eq!(bytes.len(), 8);
    assert_eq!(bytes, u64::MAX.to_le_bytes());
    assert_eq!(U64Scalar::deserialize_bytes(&bytes).unwrap(), present);
}

#[test]
fn deserialize_errors_are_guided() {
    // The only decode failure is value bytes that don't fit the data type's width.
    for bad in [&[][..], &[9][..], &[0, 0, 0][..]] {
        let err = I64Scalar::deserialize_bytes(bad).unwrap_err();
        assert!(matches!(err, ScalarError::Dtype(_)));
    }
    assert!(I64Scalar::deserialize_bytes(&[0, 0, 0])
        .unwrap_err()
        .to_string()
        .contains("8-byte"));
}

#[test]
fn float_value_semantics_are_bitwise() {
    // 0.0 and -0.0 are distinct (different bits).
    assert_ne!(F64Scalar::new(0.0), F64Scalar::new(-0.0));
    // Two NaNs with the same bit pattern are equal (byte-based equality).
    assert_eq!(F64Scalar::new(f64::NAN), F64Scalar::new(f64::NAN));
}

#[test]
fn value_semantics_as_set_keys() {
    use std::collections::HashSet;

    assert_eq!(I64Scalar::new(5), I64Scalar::new(5));
    assert_ne!(I64Scalar::new(5), I64Scalar::new(6));

    let mut set = HashSet::new();
    set.insert(I64Scalar::new(5));
    set.insert(I64Scalar::new(5));
    set.insert(I64Scalar::new(6));
    assert_eq!(set.len(), 2);
}

#[test]
fn boolean_scalar() {
    assert!(BooleanScalar::new(true).value());
    assert_eq!(BooleanScalar::new(false).serialize_bytes(), vec![0]);
    assert_eq!(BooleanScalar::new(true).serialize_bytes(), vec![1]);
    assert_eq!(
        BooleanScalar::deserialize_bytes(&[1]).unwrap(),
        BooleanScalar::new(true)
    );
}

#[test]
fn null_scalar() {
    use std::collections::HashSet;
    use yggdryl_scalar::NullScalar;

    let value = NullScalar::new();
    assert_eq!(value.value(), ());
    assert_eq!(value.arrow_data_type(), ArrowDataType::Null);
    assert_eq!(TypedScalar::data_type(&value).name(), "null");

    // Serialises to zero bytes and round-trips; any bytes are a guided error.
    assert!(value.serialize_bytes().is_empty());
    assert_eq!(NullScalar::deserialize_bytes(&[]).unwrap(), value);
    assert!(NullScalar::deserialize_bytes(&[0]).is_err());

    // All null scalars are equal and hash equal (rule 7).
    assert_eq!(NullScalar::new(), NullScalar);
    let set: HashSet<_> = [NullScalar::new(), NullScalar::new()].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn default_scalar_and_any() {
    use yggdryl_scalar::{F64Scalar, NullScalar};

    // The default scalar wraps the data type's default value.
    assert_eq!(I64Scalar::default_scalar(), I64Scalar::new(0));
    assert_eq!(F64Scalar::default_scalar(), F64Scalar::new(0.0));
    assert_eq!(NullScalar::default_scalar(), NullScalar::new());

    // default_any_scalar returns the same-typed default behind a dyn Scalar.
    let boxed = I64Scalar::new(5).default_any_scalar();
    assert_eq!(boxed.serialize_bytes(), I64Scalar::new(0).serialize_bytes());
    assert_eq!(boxed.arrow_data_type(), ArrowDataType::Int64);
}
