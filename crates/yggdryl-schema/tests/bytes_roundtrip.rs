//! Every value type round-trips through its canonical byte encoding, and
//! decoding validates fully.

use std::sync::Arc;

use yggdryl_schema::{
    AnyDataType, AnyTime32Unit, AnyTimeUnit, BooleanType, DataType, DataTypeError, DataTypeId,
    Decimal128Type, Decimal256Type, DecimalType, Duration, DurationType, Field, FieldError,
    FixedSizeBinaryType, Float64Type, Int32Type, ListType, MapType, Millisecond, Nanosecond,
    Second, StructType, Time, Time32Type, Time64Type, TimeUnit, TimeUnitId, Timestamp,
    TimestampType, TypedField, Utf8Type,
};

fn assert_roundtrip<T: DataType>(value: T) {
    assert_eq!(T::from_bytes(&value.to_bytes()), Ok(value));
}

#[test]
fn unit_types_roundtrip_as_their_id_tag() {
    // The encoding of a parameter-free type is exactly its DataTypeId tag.
    assert_eq!(BooleanType.to_bytes(), vec![DataTypeId::Boolean.to_u8()]);
    assert_roundtrip(BooleanType);
    assert_roundtrip(Int32Type);
    assert_roundtrip(Utf8Type);
    // A payload tagged with another type's id is rejected, as is trailing
    // data after the tag.
    assert_eq!(
        Int32Type::from_bytes(&BooleanType.to_bytes()),
        Err(DataTypeError::TypeIdMismatch {
            expected: DataTypeId::Int32,
            actual: DataTypeId::Boolean
        })
    );
    assert_eq!(
        Int32Type::from_bytes(&[DataTypeId::Int32.to_u8(), 7]),
        Err(DataTypeError::InvalidByteLength {
            expected: 0,
            actual: 1
        })
    );
}

#[test]
fn parameterized_types_roundtrip() {
    assert_roundtrip(Decimal128Type::from_parts(38, -10).unwrap());
    assert_roundtrip(Decimal256Type::from_parts(76, 76).unwrap());
    assert_roundtrip(FixedSizeBinaryType::from_parts(16).unwrap());
    assert_roundtrip(Time32Type::from_parts(Second));
    assert_roundtrip(Time64Type::from_parts(Nanosecond));
    assert_roundtrip(Time32Type::from_parts(
        AnyTime32Unit::from_unit_id(TimeUnitId::Millisecond).unwrap(),
    ));
    // Every unit — including the ones Arrow lacks — round-trips through the
    // erased timestamp and duration, and unit ids round-trip on their own.
    for id in 0..=10u8 {
        let unit = TimeUnitId::from_u8(id).unwrap();
        assert_eq!(TimeUnitId::from_bytes(&unit.to_bytes()), Ok(unit));
        let unit = AnyTimeUnit::from(unit);
        assert_roundtrip(DurationType::from_parts(unit));
        assert_roundtrip(TimestampType::from_parts(unit, None));
        assert_roundtrip(TimestampType::from_parts(unit, Some("Europe/Paris".into())));
    }
    // A typed unit round-trips its own tag and rejects another unit's; an
    // empty-string timezone stays distinct from no timezone.
    assert_roundtrip(TimestampType::from_parts(Millisecond, None));
    assert!(matches!(
        TimestampType::<Millisecond>::from_bytes(&[
            DataTypeId::Timestamp.to_u8(),
            TimeUnitId::Year.to_u8(),
            0,
        ]),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
    assert_roundtrip(TimestampType::from_parts(Millisecond, Some("".into())));
}

#[test]
fn decoding_validates_payloads() {
    assert!(matches!(
        Decimal128Type::from_bytes(&[DataTypeId::Decimal128.to_u8(), 1]),
        Err(DataTypeError::InvalidByteLength { expected: 2, .. })
    ));
    // The decoded parts are re-validated, not trusted.
    assert!(matches!(
        Decimal128Type::from_bytes(&[DataTypeId::Decimal128.to_u8(), 39, 0]),
        Err(DataTypeError::PrecisionOutOfRange { .. })
    ));
    assert!(matches!(
        TimeUnitId::from_bytes(&[99]),
        Err(DataTypeError::UnknownTimeUnitId { id: 99, .. })
    ));
    assert!(matches!(
        Time32Type::<Second>::from_bytes(&[
            DataTypeId::Time32.to_u8(),
            TimeUnitId::Nanosecond.to_u8(),
        ]),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
    assert!(matches!(
        Time32Type::<AnyTime32Unit>::from_bytes(&[
            DataTypeId::Time32.to_u8(),
            TimeUnitId::Nanosecond.to_u8(),
        ]),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
    let ts = DataTypeId::Timestamp.to_u8();
    assert!(matches!(
        TimestampType::<AnyTimeUnit>::from_bytes(&[ts, 0]),
        Err(DataTypeError::InvalidByteLength { expected: 2, .. })
    ));
    assert!(matches!(
        TimestampType::<AnyTimeUnit>::from_bytes(&[ts, 0, 2]),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    assert!(matches!(
        TimestampType::<AnyTimeUnit>::from_bytes(&[ts, 0, 1, 0xFF]),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    assert!(matches!(
        TimestampType::<AnyTimeUnit>::from_bytes(&[ts, 0, 0, b'x']),
        Err(DataTypeError::InvalidBytes { .. })
    ));
    let mut negative = vec![DataTypeId::FixedSizeBinary.to_u8()];
    negative.extend_from_slice(&i32::MIN.to_le_bytes());
    assert!(matches!(
        FixedSizeBinaryType::from_bytes(&negative),
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
    let field = TypedField::from_parts("reading", Float64Type, true, metadata);
    assert_eq!(TypedField::from_bytes(&field.to_bytes()), Ok(field));

    let empty_name = TypedField::from_parts("", Int32Type, false, Default::default());
    assert_eq!(
        TypedField::from_bytes(&empty_name.to_bytes()),
        Ok(empty_name)
    );
}

#[test]
fn field_decoding_validates_payloads() {
    let field = TypedField::from_parts("id", Int32Type, false, Default::default());
    let encoded = field.to_bytes();

    assert!(matches!(
        TypedField::<Int32Type>::from_bytes(&[]),
        Err(FieldError::InvalidBytes { .. })
    ));
    assert!(matches!(
        TypedField::<Int32Type>::from_bytes(&encoded[..encoded.len() - 1]),
        Err(FieldError::InvalidBytes { .. })
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        TypedField::<Int32Type>::from_bytes(&trailing),
        Err(FieldError::InvalidBytes { .. })
    ));
    let mut bad_flag = encoded;
    bad_flag[0] = 2;
    assert!(matches!(
        TypedField::<Int32Type>::from_bytes(&bad_flag),
        Err(FieldError::InvalidBytes { .. })
    ));
}

#[test]
fn nested_lists_roundtrip_through_bytes() {
    let item = Arc::new(TypedField::from_parts(
        "item",
        Int32Type,
        true,
        Default::default(),
    ));
    let outer = ListType::from_parts(Arc::new(TypedField::from_parts(
        "rows",
        ListType::from_parts(item),
        false,
        Default::default(),
    )));
    assert_eq!(ListType::from_bytes(&outer.to_bytes()), Ok(outer));
}

#[test]
fn structs_maps_and_erased_types_roundtrip_through_bytes() {
    let person = StructType::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "id",
            Int32Type.into(),
            false,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "name",
            Utf8Type.into(),
            true,
            Default::default(),
        )),
    ]);
    assert_roundtrip(person.clone());
    assert_roundtrip(StructType::from_parts(vec![]));

    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person.clone(),
        false,
        Default::default(),
    ));
    let map = MapType::from_parts(entries, true).unwrap();
    assert_roundtrip(map.clone());
    // The sorted flag (right after the id tag) is validated on decode.
    let mut corrupted = map.to_bytes();
    corrupted[1] = 9;
    assert!(matches!(
        MapType::from_bytes(&corrupted),
        Err(DataTypeError::InvalidBytes { .. })
    ));

    // The erased encoding is the concrete payload behind the type id tag.
    let any = AnyDataType::from(map);
    assert_eq!(any.to_bytes()[0], any.type_id().to_u8());
    assert_roundtrip(any);
    assert_roundtrip(AnyDataType::from(TimestampType::from_parts(
        Millisecond,
        Some("UTC".into()),
    )));
    assert!(matches!(
        AnyDataType::from_bytes(&[200]),
        Err(DataTypeError::UnknownTypeId { .. })
    ));
}
