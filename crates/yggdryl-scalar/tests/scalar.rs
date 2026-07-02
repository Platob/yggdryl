//! `Scalar` construction, typed access, validation and byte round-trips.

use arrow_buffer::Buffer;
use yggdryl_scalar::{Scalar, ScalarError};
use yggdryl_schema::Duration;
use yggdryl_schema::{
    BinaryType, BooleanType, Decimal128Type, DecimalType, DurationType, FixedSizeBinaryType,
    Float32Type as Float32, Float64Type, Int32Type, Int64Type, LargeUtf8Type, Millisecond, Minute,
    Nanosecond, Second, Time, Time32Type, Time64Type, Timestamp, TimestampType, Utf8Type, Week,
    Year,
};

#[test]
fn native_scalars_roundtrip_their_value() {
    assert_eq!(Scalar::from_native(Int32Type, -7).as_native(), Some(-7));
    assert_eq!(Scalar::from_native(Float64Type, 1.5).as_native(), Some(1.5));
    assert_eq!(
        Scalar::from_native(Decimal128Type::from_parts(38, 2).unwrap(), 123i128).as_native(),
        Some(123),
    );
    let timestamp = TimestampType::from_parts(Millisecond, Some("UTC".into()));
    assert_eq!(
        Scalar::from_native(timestamp, 1_700_000_000_000i64).as_native(),
        Some(1_700_000_000_000),
    );

    assert!(Scalar::null(Int32Type).is_null());
    assert_eq!(Scalar::<Int32Type>::null(Int32Type).as_native(), None);
}

#[test]
fn temporal_scalars_exist_for_every_unit_typed_implementation() {
    // Timestamps and durations of any unit — native or anchored — plus both
    // time widths and both dates hold scalars.
    assert_eq!(
        Scalar::from_native(TimestampType::from_parts(Year, None), 55i64).as_native(),
        Some(55),
    );
    assert_eq!(
        Scalar::from_native(DurationType::from_parts(Minute), 90i64).as_native(),
        Some(90),
    );
    assert_eq!(
        Scalar::from_native(DurationType::from_parts(Nanosecond), 1_000i64).as_native(),
        Some(1_000),
    );
    assert_eq!(
        Scalar::from_native(Time32Type::from_parts(Second), 43_200i32).as_native(),
        Some(43_200),
    );
    assert_eq!(
        Scalar::from_native(Time64Type::from_parts(Nanosecond), 1_000_000i64).as_native(),
        Some(1_000_000),
    );
    assert_eq!(
        Scalar::from_native(yggdryl_schema::Date32Type, 20_000i32).as_native(),
        Some(20_000),
    );
    assert_eq!(
        Scalar::from_native(yggdryl_schema::Date64Type, 1_700_000_000_000i64).as_native(),
        Some(1_700_000_000_000),
    );
    // A 32-bit time scalar really is 32-bit: 8 bytes are rejected.
    assert!(Scalar::from_parts(
        Time32Type::from_parts(Second),
        Some(arrow_buffer::Buffer::from(0i64.to_le_bytes().to_vec())),
    )
    .is_err());
    assert!(Scalar::null(TimestampType::from_parts(Week, None)).is_null());
}

#[test]
fn boolean_string_and_binary_scalars_roundtrip() {
    assert_eq!(Scalar::from_bool(true).as_bool(), Some(true));
    assert_eq!(Scalar::from_bool(false).as_bool(), Some(false));
    assert_eq!(Scalar::null(BooleanType).as_bool(), None);

    assert_eq!(Scalar::from_string(Utf8Type, "ygg").as_str(), Some("ygg"));
    assert_eq!(Scalar::from_string(LargeUtf8Type, "").as_str(), Some(""));

    let bytes = Scalar::from_binary(BinaryType, [0xDE, 0xAD]).unwrap();
    assert_eq!(bytes.as_binary(), Some(&[0xDE, 0xAD][..]));
    let uuid_type = FixedSizeBinaryType::from_parts(16).unwrap();
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
        Scalar::from_parts(Int32Type, Some(Buffer::from(vec![0u8; 3]))),
        Err(ScalarError::InvalidByteLength {
            expected: 4,
            actual: 3
        })
    );
    // A buffer sliced off the element grid is rejected, not misread.
    let misaligned = Buffer::from(vec![0u8; 5]).slice(1);
    assert!(matches!(
        Scalar::from_parts(Int32Type, Some(misaligned)),
        Err(ScalarError::MisalignedBuffer { .. } | ScalarError::InvalidByteLength { .. })
    ));
    assert_eq!(
        Scalar::from_parts(BooleanType, Some(Buffer::from(vec![2u8]))),
        Err(ScalarError::InvalidBoolean { value: 2 })
    );
    assert_eq!(
        Scalar::from_parts(Utf8Type, Some(Buffer::from(vec![0xFF]))),
        Err(ScalarError::InvalidUtf8)
    );
    let uuid_type = FixedSizeBinaryType::from_parts(16).unwrap();
    assert!(Scalar::from_binary(uuid_type, [0u8; 4]).is_err());
}

#[test]
fn scalars_roundtrip_through_bytes() {
    let values = [
        Scalar::from_native(Int32Type, i32::MIN),
        Scalar::null(Int32Type),
    ];
    for value in values {
        assert_eq!(Scalar::from_bytes(&value.to_bytes()), Ok(value));
    }
    let name = Scalar::from_string(Utf8Type, "zero-copy");
    assert_eq!(Scalar::from_bytes(&name.to_bytes()), Ok(name));

    // Corrupted payloads are rejected with typed errors.
    assert!(Scalar::<Int32Type>::from_bytes(&[1, 2, 3]).is_err());
    let mut bad_flag = Scalar::from_native(Int32Type, 1).to_bytes();
    let flag_at = bad_flag.len() - 5;
    bad_flag[flag_at] = 9;
    assert!(matches!(
        Scalar::<Int32Type>::from_bytes(&bad_flag),
        Err(ScalarError::InvalidBytes { .. })
    ));
}

#[test]
fn equality_and_hashing_are_content_based() {
    use std::collections::HashSet;

    let a = Scalar::from_native(Int32Type, 7);
    let b = Scalar::from_bytes(&a.to_bytes()).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, Scalar::from_native(Int32Type, 8));
    assert_ne!(a, Scalar::null(Int32Type));

    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}

#[test]
fn checked_widening_accessors_never_lie() {
    use yggdryl_schema::{Int8Type, UInt64Type};

    // The matching width comes back as-is; narrower natives widen.
    assert_eq!(Scalar::from_native(Int64Type, 42i64).as_i64(), Some(42));
    assert_eq!(Scalar::from_native(Int8Type, -7i8).as_i64(), Some(-7));
    assert_eq!(Scalar::from_native(Int8Type, -7i8).as_i128(), Some(-7));
    assert_eq!(Scalar::from_native(Float32, 1.5f32).as_f64(), Some(1.5));
    assert_eq!(Scalar::from_native(Int32Type, 3).as_f64(), Some(3.0));

    // Value-dependent conversions are checked, never truncated.
    assert_eq!(
        Scalar::from_native(UInt64Type, u64::MAX).as_i64(),
        None // does not fit an i64
    );
    assert_eq!(Scalar::from_native(UInt64Type, 7u64).as_i64(), Some(7));
    assert_eq!(Scalar::from_native(Int64Type, -1i64).as_u64(), None);
    assert_eq!(
        Scalar::from_native(Decimal128Type::from_parts(38, 0).unwrap(), 5i128).as_i128(),
        Some(5),
    );

    // Null propagates through every accessor.
    assert_eq!(Scalar::null(Int64Type).as_i64(), None);
}

#[test]
fn per_type_scalars_have_their_own_implementations() {
    use yggdryl_scalar::{
        BooleanScalar, Decimal128Scalar, FixedSizeBinaryScalar, Int64Scalar, TimestampScalar,
        UInt64Scalar, Utf8Scalar,
    };

    // Parameter-free constructors drop the data-type argument entirely.
    assert_eq!(Int64Scalar::from_native(42).as_i64(), Some(42));
    assert!(Int64Scalar::null().is_null());
    assert_eq!(UInt64Scalar::from_native(7).as_u64(), Some(7));
    assert_eq!(BooleanScalar::from_bool(true).as_bool(), Some(true));
    assert_eq!(Utf8Scalar::from_string("ygg").as_str(), Some("ygg"));

    // Parameterized ones take their type first, like the engine.
    let decimal = Decimal128Type::from_parts(38, 2).unwrap();
    assert_eq!(
        Decimal128Scalar::from_native(decimal, 123).as_i128(),
        Some(123)
    );
    let uuid_type = FixedSizeBinaryType::from_parts(4).unwrap();
    assert!(FixedSizeBinaryScalar::from_binary(uuid_type, [1, 2]).is_err());
    let ts = TimestampScalar::from_native(
        TimestampType::from_parts(Millisecond, Some("UTC".into())),
        1_700_000_000_000,
    );
    assert_eq!(ts.as_i64(), Some(1_700_000_000_000));

    // Family members round-trip bytes and convert to and from the engine.
    let scalar = Int64Scalar::from_native(9);
    assert_eq!(
        Int64Scalar::from_bytes(&scalar.to_bytes()),
        Ok(scalar.clone())
    );
    let engine: Scalar<Int64Type> = scalar.clone().into();
    assert_eq!(Int64Scalar::from(engine), scalar);
}
