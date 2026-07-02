//! `Scalar` construction, typed access, validation and byte round-trips.

use arrow_buffer::Buffer;
use yggdryl_scalar::{Scalar, ScalarError};
use yggdryl_schema::{
    Binary, Boolean, Decimal128, FixedSizeBinary, Float64, Int32, LargeUtf8, TimeUnit, Timestamp,
    Utf8,
};

#[test]
fn native_scalars_roundtrip_their_value() {
    assert_eq!(Scalar::from_native(Int32, -7).as_native(), Some(-7));
    assert_eq!(Scalar::from_native(Float64, 1.5).as_native(), Some(1.5));
    assert_eq!(
        Scalar::from_native(Decimal128::from_parts(38, 2).unwrap(), 123i128).as_native(),
        Some(123),
    );
    let timestamp = Timestamp::from_parts(TimeUnit::Millisecond, Some("UTC".into()));
    assert_eq!(
        Scalar::from_native(timestamp, 1_700_000_000_000i64).as_native(),
        Some(1_700_000_000_000),
    );

    assert!(Scalar::null(Int32).is_null());
    assert_eq!(Scalar::<Int32>::null(Int32).as_native(), None);
}

#[test]
fn boolean_string_and_binary_scalars_roundtrip() {
    assert_eq!(Scalar::from_bool(true).as_bool(), Some(true));
    assert_eq!(Scalar::from_bool(false).as_bool(), Some(false));
    assert_eq!(Scalar::null(Boolean).as_bool(), None);

    assert_eq!(Scalar::from_string(Utf8, "ygg").as_str(), Some("ygg"));
    assert_eq!(Scalar::from_string(LargeUtf8, "").as_str(), Some(""));

    let bytes = Scalar::from_binary(Binary, [0xDE, 0xAD]).unwrap();
    assert_eq!(bytes.as_binary(), Some(&[0xDE, 0xAD][..]));
    let uuid_type = FixedSizeBinary::from_parts(16).unwrap();
    assert_eq!(
        Scalar::from_binary(uuid_type, [7u8; 16])
            .unwrap()
            .as_binary(),
        Some(&[7u8; 16][..]),
    );
}

#[test]
fn construction_validates_the_layout() {
    assert_eq!(
        Scalar::from_parts(Int32, Some(Buffer::from(vec![0u8; 3]))),
        Err(ScalarError::InvalidByteLength {
            expected: 4,
            actual: 3
        })
    );
    // A buffer sliced off the element grid is rejected, not misread.
    let misaligned = Buffer::from(vec![0u8; 5]).slice(1);
    assert!(matches!(
        Scalar::from_parts(Int32, Some(misaligned)),
        Err(ScalarError::MisalignedBuffer { .. } | ScalarError::InvalidByteLength { .. })
    ));
    assert_eq!(
        Scalar::from_parts(Boolean, Some(Buffer::from(vec![2u8]))),
        Err(ScalarError::InvalidBoolean { value: 2 })
    );
    assert_eq!(
        Scalar::from_parts(Utf8, Some(Buffer::from(vec![0xFF]))),
        Err(ScalarError::InvalidUtf8)
    );
    let uuid_type = FixedSizeBinary::from_parts(16).unwrap();
    assert!(Scalar::from_binary(uuid_type, [0u8; 4]).is_err());
}

#[test]
fn scalars_roundtrip_through_bytes() {
    let values = [Scalar::from_native(Int32, i32::MIN), Scalar::null(Int32)];
    for value in values {
        assert_eq!(Scalar::from_bytes(&value.to_bytes()), Ok(value));
    }
    let name = Scalar::from_string(Utf8, "zero-copy");
    assert_eq!(Scalar::from_bytes(&name.to_bytes()), Ok(name));

    // Corrupted payloads are rejected with typed errors.
    assert!(Scalar::<Int32>::from_bytes(&[1, 2, 3]).is_err());
    let mut bad_flag = Scalar::from_native(Int32, 1).to_bytes();
    let flag_at = bad_flag.len() - 5;
    bad_flag[flag_at] = 9;
    assert!(matches!(
        Scalar::<Int32>::from_bytes(&bad_flag),
        Err(ScalarError::InvalidBytes { .. })
    ));
}

#[test]
fn equality_and_hashing_are_content_based() {
    use std::collections::HashSet;

    let a = Scalar::from_native(Int32, 7);
    let b = Scalar::from_bytes(&a.to_bytes()).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, Scalar::from_native(Int32, 8));
    assert_ne!(a, Scalar::null(Int32));

    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}
