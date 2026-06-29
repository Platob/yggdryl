//! Tests for the [`Binary`] scalar across every current (binary) data type.

use yggdryl_scalar::{Binary, Scalar};
use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, FixedSizeBinaryType, LargeBinaryType,
    LargeBinaryViewType,
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
