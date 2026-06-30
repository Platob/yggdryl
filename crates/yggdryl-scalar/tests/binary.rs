//! Tests for the [`Binary`] scalar across every current (binary) data type.

use yggdryl_scalar::{Binary, Scalar, ScalarError};
use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, LargeBinaryType, LargeBinaryViewType,
};

#[test]
fn encode_decode_native_values() {
    let s = Binary::encode(BinaryType::new(), "héllo");
    assert_eq!(s.as_bytes(), "héllo".as_bytes());
    assert_eq!(s.decode::<String>().unwrap(), "héllo");
    assert_eq!(s.decode::<Vec<u8>>().unwrap(), "héllo".as_bytes().to_vec());

    // Raw bytes round-trip too.
    let raw = Binary::encode(BinaryType::new(), b"\x00\x01".as_slice());
    assert_eq!(raw.decode::<Vec<u8>>().unwrap(), vec![0, 1]);

    // Invalid UTF-8 fails to decode as a string.
    let bad = Binary::new(BinaryType::new(), vec![0xff, 0xfe]);
    assert_eq!(bad.decode::<String>(), Err(ScalarError::NonUtf8));
}

#[test]
fn cast_re_tags_bytes() {
    let s = Binary::new(BinaryType::new(), b"hi".to_vec());
    // Casting to another binary type keeps the bytes and swaps the type.
    let large = s.cast(LargeBinaryType::new());
    assert_eq!(large.dtype().type_id(), DataTypeId::LargeBinary);
    assert_eq!(large.as_bytes(), b"hi");
    // Casting to a capped type truncates.
    assert_eq!(s.cast(BinaryType::new().with_byte_size(1)).as_bytes(), b"h");
    // Casting to the same type leaves the bytes unchanged.
    assert_eq!(s.cast(BinaryType::new()).as_bytes(), b"hi");
}

#[test]
fn view_scalars_share_the_buffer() {
    let s = Binary::new(BinaryViewType::new(), b"hello".to_vec());
    let base = s.as_bytes().as_ptr();

    // Cloning shares the allocation (no deep copy) — same address.
    assert_eq!(s.clone().as_bytes().as_ptr(), base);

    // Casting between view types shares it too.
    let cast = s.cast(LargeBinaryViewType::new());
    assert_eq!(cast.dtype().type_id(), DataTypeId::LargeBinaryView);
    assert_eq!(cast.as_bytes().as_ptr(), base);

    // A truncating cast still shares the allocation (a zero-copy slice).
    let truncated = s.cast(BinaryViewType::new().with_byte_size(2));
    assert_eq!(truncated.as_bytes(), b"he");
    assert_eq!(truncated.as_bytes().as_ptr(), base);
}

#[test]
fn dtype_for_all_binary_types() {
    assert_eq!(
        Binary::new(BinaryType::new(), b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::Binary
    );
    assert_eq!(
        Binary::new(LargeBinaryType::new(), b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::LargeBinary
    );
    assert_eq!(
        Binary::new(BinaryViewType::new(), b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::BinaryView
    );
    assert_eq!(
        Binary::new(LargeBinaryViewType::new(), b"a".to_vec())
            .dtype()
            .type_id(),
        DataTypeId::LargeBinaryView
    );
}

#[test]
fn to_from_bytes_round_trip() {
    let value = Binary::new(BinaryType::new(), b"hello".to_vec());
    assert_eq!(value.to_bytes(), b"hello".to_vec());
    assert_eq!(value.as_bytes(), b"hello");
    let rebuilt = Binary::from_bytes(BinaryType::new(), &value.to_bytes());
    assert_eq!(rebuilt, value);
}

#[test]
fn is_fixed_size() {
    assert!(Binary::new(BinaryType::new().with_byte_size(2), b"ab".to_vec()).is_fixed_size());
    assert!(!Binary::new(BinaryType::new(), b"ab".to_vec()).is_fixed_size());
}

#[test]
fn over_long_payload_is_truncated() {
    // A capped type truncates anything beyond its cap.
    let capped = Binary::new(BinaryType::new().with_byte_size(3), b"hello".to_vec());
    assert_eq!(capped.as_bytes(), b"hel");
    // from_bytes applies the same rule.
    let via_bytes = Binary::from_bytes(BinaryType::new().with_byte_size(1), b"hello");
    assert_eq!(via_bytes.as_bytes(), b"h");
    // Unbounded types keep every byte.
    let unbounded = Binary::new(BinaryType::new(), b"hello".to_vec());
    assert_eq!(unbounded.as_bytes(), b"hello");
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let value = Binary::new(BinaryType::new().with_byte_size(2), vec![1u8, 2]);
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<Binary<BinaryType>>(&json).unwrap(),
        value
    );
}
