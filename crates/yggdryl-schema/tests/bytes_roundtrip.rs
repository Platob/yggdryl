//! Every value type round-trips through its canonical byte encoding, and
//! decoding validates fully.

use std::sync::Arc;

use yggdryl_schema::{
    AnyDataType, AnyTimeUnit, Boolean, DataType, DataTypeError, Decimal128, Decimal256, Duration,
    Field, FieldError, FixedSizeBinary, Float64, Int32, List, Map, Millisecond, Struct, Time32,
    Time64, TimeUnitId, Timestamp, TypedField, Utf8,
};

fn assert_roundtrip<T: DataType>(value: T) {
    assert_eq!(T::from_bytes(&value.to_bytes()), Ok(value));
}

#[test]
fn unit_types_roundtrip_as_empty_payloads() {
    assert_eq!(Boolean.to_bytes(), Vec::<u8>::new());
    assert_roundtrip(Boolean);
    assert_roundtrip(Int32);
    assert_roundtrip(Utf8);
    assert_eq!(
        Int32::from_bytes(&[0]),
        Err(DataTypeError::InvalidByteLength {
            expected: 0,
            actual: 1
        })
    );
}

#[test]
fn parameterized_types_roundtrip() {
    assert_roundtrip(Decimal128::from_parts(38, -10).unwrap());
    assert_roundtrip(Decimal256::from_parts(76, 76).unwrap());
    assert_roundtrip(FixedSizeBinary::from_parts(16).unwrap());
    assert_roundtrip(Time32::from_parts(TimeUnitId::Second).unwrap());
    assert_roundtrip(Time64::from_parts(TimeUnitId::Nanosecond).unwrap());
    for unit in [
        TimeUnitId::Second,
        TimeUnitId::Millisecond,
        TimeUnitId::Microsecond,
        TimeUnitId::Nanosecond,
    ] {
        assert_roundtrip(Duration::from_parts(unit).unwrap());
    }
    // Every unit — including the ones Arrow lacks — round-trips through the
    // erased timestamp, and unit ids round-trip on their own.
    for id in 0..=10u8 {
        let unit = TimeUnitId::from_u8(id).unwrap();
        assert_eq!(TimeUnitId::from_bytes(&unit.to_bytes()), Ok(unit));
        let unit = AnyTimeUnit::from(unit);
        assert_roundtrip(Timestamp::from_parts(unit, None));
        assert_roundtrip(Timestamp::from_parts(unit, Some("Europe/Paris".into())));
    }
    // A typed unit round-trips its own tag and rejects another unit's; an
    // empty-string timezone stays distinct from no timezone.
    assert_roundtrip(Timestamp::from_parts(Millisecond, None));
    assert!(matches!(
        Timestamp::<Millisecond>::from_bytes(&[TimeUnitId::Year.to_u8(), 0]),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
    assert_roundtrip(Timestamp::from_parts(Millisecond, Some("".into())));
}

#[test]
fn decoding_validates_payloads() {
    assert!(matches!(
        Decimal128::from_bytes(&[1]),
        Err(DataTypeError::InvalidByteLength { expected: 2, .. })
    ));
    // The decoded parts are re-validated, not trusted.
    assert!(matches!(
        Decimal128::from_bytes(&[39, 0]),
        Err(DataTypeError::PrecisionOutOfRange { .. })
    ));
    assert!(matches!(
        TimeUnitId::from_bytes(&[99]),
        Err(DataTypeError::UnknownTimeUnitId { id: 99, .. })
    ));
    assert!(matches!(
        Time32::from_bytes(&TimeUnitId::Nanosecond.to_bytes()),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
    assert!(matches!(
        Timestamp::<AnyTimeUnit>::from_bytes(&[0]),
        Err(DataTypeError::InvalidByteLength { expected: 2, .. })
    ));
    assert!(matches!(
        Timestamp::<AnyTimeUnit>::from_bytes(&[0, 2]),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    assert!(matches!(
        Timestamp::<AnyTimeUnit>::from_bytes(&[0, 1, 0xFF]),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    assert!(matches!(
        Timestamp::<AnyTimeUnit>::from_bytes(&[0, 0, b'x']),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    assert!(matches!(
        FixedSizeBinary::from_bytes(&i32::MIN.to_le_bytes()),
        Err(DataTypeError::NegativeFixedSize { .. })
    ));
}

#[test]
fn fields_roundtrip_with_every_part() {
    let metadata = [
        ("a".to_string(), "1".to_string()),
        ("b".to_string(), "".to_string()),
    ]
    .into_iter()
    .collect();
    let field = TypedField::from_parts("reading", Float64, true, metadata);
    assert_eq!(TypedField::from_bytes(&field.to_bytes()), Ok(field));

    let empty_name = TypedField::from_parts("", Int32, false, Default::default());
    assert_eq!(
        TypedField::from_bytes(&empty_name.to_bytes()),
        Ok(empty_name)
    );
}

#[test]
fn field_decoding_validates_payloads() {
    let field = TypedField::from_parts("id", Int32, false, Default::default());
    let encoded = field.to_bytes();

    assert!(matches!(
        TypedField::<Int32>::from_bytes(&[]),
        Err(FieldError::InvalidBytes { .. })
    ));
    assert!(matches!(
        TypedField::<Int32>::from_bytes(&encoded[..encoded.len() - 1]),
        Err(FieldError::InvalidBytes { .. })
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        TypedField::<Int32>::from_bytes(&trailing),
        Err(FieldError::InvalidBytes { .. })
    ));
    let mut bad_flag = encoded;
    bad_flag[0] = 2;
    assert!(matches!(
        TypedField::<Int32>::from_bytes(&bad_flag),
        Err(FieldError::InvalidBytes { .. })
    ));
}

#[test]
fn nested_lists_roundtrip_through_bytes() {
    let item = Arc::new(TypedField::from_parts(
        "item",
        Int32,
        true,
        Default::default(),
    ));
    let outer = List::from_parts(Arc::new(TypedField::from_parts(
        "rows",
        List::from_parts(item),
        false,
        Default::default(),
    )));
    assert_eq!(List::from_bytes(&outer.to_bytes()), Ok(outer));
}

#[test]
fn structs_maps_and_erased_types_roundtrip_through_bytes() {
    let person = Struct::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "id",
            Int32.into(),
            false,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "name",
            Utf8.into(),
            true,
            Default::default(),
        )),
    ]);
    assert_roundtrip(person.clone());
    assert_roundtrip(Struct::from_parts(vec![]));

    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person.clone(),
        false,
        Default::default(),
    ));
    let map = Map::from_parts(entries, true).unwrap();
    assert_roundtrip(map.clone());
    // The sorted flag is validated on decode.
    let mut corrupted = map.to_bytes();
    corrupted[0] = 9;
    assert!(matches!(
        Map::from_bytes(&corrupted),
        Err(DataTypeError::InvalidBytes { .. })
    ));

    // The erased encoding is the concrete payload behind the type id tag.
    let any = AnyDataType::from(map);
    assert_eq!(any.to_bytes()[0], any.type_id().to_u8());
    assert_roundtrip(any);
    assert_roundtrip(AnyDataType::from(Timestamp::from_parts(
        Millisecond,
        Some("UTC".into()),
    )));
    assert!(matches!(
        AnyDataType::from_bytes(&[200]),
        Err(DataTypeError::UnknownTypeId { .. })
    ));
}
