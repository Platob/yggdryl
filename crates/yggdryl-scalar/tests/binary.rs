//! Tests for the [`Binary`] scalar across every current (binary) data type.

use yggdryl_scalar::{Binary, Scalar};
use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, FixedSizeBinaryType, LargeBinaryType,
    LargeBinaryViewType, MaxedSizeBinaryType,
};

#[test]
fn dtype_for_all_binary_types() {
    assert_eq!(
        Binary::new(BinaryType, b"a".to_vec()).dtype().type_id(),
        DataTypeId::Binary
    );
    assert_eq!(
        Binary::new(LargeBinaryType, b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::LargeBinary
    );
    assert_eq!(
        Binary::new(BinaryViewType, b"a".to_vec()).dtype().type_id(),
        DataTypeId::BinaryView
    );
    assert_eq!(
        Binary::new(LargeBinaryViewType, b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::LargeBinaryView
    );
    assert_eq!(
        Binary::new(FixedSizeBinaryType::new(1), b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::FixedSizeBinary
    );
}

#[test]
fn to_from_bytes_round_trip() {
    let value = Binary::new(BinaryType, b"hello".to_vec());
    assert_eq!(value.to_bytes(), b"hello".to_vec());
    assert_eq!(value.as_bytes(), b"hello");
    let rebuilt = Binary::from_bytes(BinaryType, &value.to_bytes());
    assert_eq!(rebuilt, value);
}

#[test]
fn is_fixed_size() {
    assert!(Binary::new(FixedSizeBinaryType::new(2), b"ab".to_vec()).is_fixed_size());
    assert!(!Binary::new(BinaryType, b"ab".to_vec()).is_fixed_size());
    assert!(!Binary::new(MaxedSizeBinaryType::new(2), b"ab".to_vec()).is_fixed_size());
}

#[test]
fn over_long_payload_is_truncated() {
    // A max-size type truncates anything beyond its cap.
    let capped = Binary::new(MaxedSizeBinaryType::new(3), b"hello".to_vec());
    assert_eq!(capped.as_bytes(), b"hel");
    // So does a fixed-size type (its exact width is also the maximum).
    let fixed = Binary::new(FixedSizeBinaryType::new(2), b"hello".to_vec());
    assert_eq!(fixed.as_bytes(), b"he");
    // from_bytes applies the same rule.
    let via_bytes = Binary::from_bytes(MaxedSizeBinaryType::new(1), b"hello");
    assert_eq!(via_bytes.as_bytes(), b"h");
    // Unbounded types keep every byte.
    let unbounded = Binary::new(BinaryType, b"hello".to_vec());
    assert_eq!(unbounded.as_bytes(), b"hello");
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let value = Binary::new(FixedSizeBinaryType::new(2), vec![1u8, 2]);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<Binary<FixedSizeBinaryType>>(&json).unwrap(),
        value
    );
}
