//! Tests for the [`Binary`] scalar across every current (binary) data type.

use yggdryl_scalar::{Binary, Scalar, ScalarError};
use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, FixedSizeBinaryType, LargeBinaryType,
    LargeBinaryViewType, MaxedSizeBinaryType,
};

#[test]
fn encode_decode_native_values() {
    let s = Binary::encode(BinaryType, "héllo");
    assert_eq!(s.as_bytes(), "héllo".as_bytes());
    assert_eq!(s.decode::<String>().unwrap(), "héllo");
    assert_eq!(s.decode::<Vec<u8>>().unwrap(), "héllo".as_bytes().to_vec());

    // Raw bytes round-trip too.
    let raw = Binary::encode(BinaryType, b"\x00\x01".as_slice());
    assert_eq!(raw.decode::<Vec<u8>>().unwrap(), vec![0, 1]);

    // Invalid UTF-8 fails to decode as a string.
    let bad = Binary::new(BinaryType, vec![0xff, 0xfe]);
    assert_eq!(bad.decode::<String>(), Err(ScalarError::NonUtf8));
}

#[test]
fn cast_re_tags_bytes() {
    let s = Binary::new(BinaryType, b"hi".to_vec());
    // Casting to another binary type keeps the bytes and swaps the type.
    let large = s.cast(LargeBinaryType);
    assert_eq!(large.dtype().type_id(), DataTypeId::LargeBinary);
    assert_eq!(large.as_bytes(), b"hi");
    // Casting to a smaller fixed type truncates.
    assert_eq!(s.cast(FixedSizeBinaryType::new(1)).as_bytes(), b"h");
    // Casting to the same type leaves the bytes unchanged.
    assert_eq!(s.cast(BinaryType).as_bytes(), b"hi");
}

#[test]
fn view_scalars_share_the_buffer() {
    let s = Binary::new(BinaryViewType, b"hello".to_vec());
    let base = s.as_bytes().as_ptr();

    // Cloning shares the allocation (no deep copy) — same address.
    assert_eq!(s.clone().as_bytes().as_ptr(), base);

    // Casting between view types shares it too.
    let cast = s.cast(LargeBinaryViewType);
    assert_eq!(cast.dtype().type_id(), DataTypeId::LargeBinaryView);
    assert_eq!(cast.as_bytes().as_ptr(), base);

    // A truncating cast still shares the allocation (a zero-copy slice).
    let truncated = s.cast(FixedSizeBinaryType::new(2));
    assert_eq!(truncated.as_bytes(), b"he");
    assert_eq!(truncated.as_bytes().as_ptr(), base);
}

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
