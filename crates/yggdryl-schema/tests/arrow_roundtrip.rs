//! The Arrow mapping is total and reversible for the supported subset: every
//! type round-trips through `to_arrow` / `from_arrow`, exhaustively over each
//! type's parameter space where one exists.

use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;
use yggdryl_schema::{
    metadata, AnyDataType, AnyTimeUnit, Binary, Boolean, DataType, DataTypeError, Date32, Date64,
    Decimal128, Decimal256, Duration, Field, FixedSizeBinary, Float32, Float64, Int16, Int32,
    Int64, Int8, LargeBinary, LargeList, LargeUtf8, List, Map, Minute, Nanosecond, Struct, Time32,
    Time64, TimeUnitId, Timestamp, TypedField, UInt16, UInt32, UInt64, UInt8, Utf8, Year,
};

const TIME_UNITS: [TimeUnitId; 4] = [
    TimeUnitId::Second,
    TimeUnitId::Millisecond,
    TimeUnitId::Microsecond,
    TimeUnitId::Nanosecond,
];

fn assert_roundtrip<T: DataType>(value: T, arrow: ArrowDataType) {
    assert_eq!(value.to_arrow(), arrow);
    assert_eq!(T::from_arrow(&arrow), Ok(value));
}

#[test]
fn unit_types_roundtrip() {
    assert_roundtrip(Boolean, ArrowDataType::Boolean);
    assert_roundtrip(Int8, ArrowDataType::Int8);
    assert_roundtrip(Int16, ArrowDataType::Int16);
    assert_roundtrip(Int32, ArrowDataType::Int32);
    assert_roundtrip(Int64, ArrowDataType::Int64);
    assert_roundtrip(UInt8, ArrowDataType::UInt8);
    assert_roundtrip(UInt16, ArrowDataType::UInt16);
    assert_roundtrip(UInt32, ArrowDataType::UInt32);
    assert_roundtrip(UInt64, ArrowDataType::UInt64);
    assert_roundtrip(Float32, ArrowDataType::Float32);
    assert_roundtrip(Float64, ArrowDataType::Float64);
    assert_roundtrip(Utf8, ArrowDataType::Utf8);
    assert_roundtrip(LargeUtf8, ArrowDataType::LargeUtf8);
    assert_roundtrip(Binary, ArrowDataType::Binary);
    assert_roundtrip(LargeBinary, ArrowDataType::LargeBinary);
    assert_roundtrip(Date32, ArrowDataType::Date32);
    assert_roundtrip(Date64, ArrowDataType::Date64);
}

#[test]
fn mismatched_arrow_type_is_rejected() {
    assert!(matches!(
        Int8::from_arrow(&ArrowDataType::Utf8),
        Err(DataTypeError::ArrowTypeMismatch {
            expected: "int8",
            ..
        })
    ));
    assert!(matches!(
        Decimal128::from_arrow(&ArrowDataType::Decimal256(10, 2)),
        Err(DataTypeError::ArrowTypeMismatch { .. })
    ));
    assert!(matches!(
        Timestamp::<Nanosecond>::from_arrow(&ArrowDataType::Date32),
        Err(DataTypeError::ArrowTypeMismatch { .. })
    ));
    assert!(matches!(
        List::<Int32>::from_arrow(&ArrowDataType::Int32),
        Err(DataTypeError::ArrowTypeMismatch { .. })
    ));
}

#[test]
fn decimal128_roundtrips_exhaustively() {
    for precision in 1..=38u8 {
        let magnitude = precision as i8;
        for scale in -magnitude..=magnitude {
            let decimal = Decimal128::from_parts(precision, scale).unwrap();
            assert_roundtrip(decimal, ArrowDataType::Decimal128(precision, scale));
        }
    }
}

#[test]
fn decimal256_roundtrips_exhaustively() {
    for precision in 1..=76u8 {
        let magnitude = precision as i8;
        for scale in -magnitude..=magnitude {
            let decimal = Decimal256::from_parts(precision, scale).unwrap();
            assert_roundtrip(decimal, ArrowDataType::Decimal256(precision, scale));
        }
    }
}

#[test]
fn decimal_validation_rejects_invalid_parts() {
    assert_eq!(
        Decimal128::from_parts(0, 0),
        Err(DataTypeError::PrecisionOutOfRange {
            precision: 0,
            max: 38
        })
    );
    assert_eq!(
        Decimal128::from_parts(39, 0),
        Err(DataTypeError::PrecisionOutOfRange {
            precision: 39,
            max: 38
        })
    );
    assert_eq!(
        Decimal128::from_parts(10, 11),
        Err(DataTypeError::ScaleOutOfRange {
            scale: 11,
            precision: 10
        })
    );
    assert_eq!(
        Decimal128::from_parts(10, -11),
        Err(DataTypeError::ScaleOutOfRange {
            scale: -11,
            precision: 10
        })
    );
    assert_eq!(
        Decimal256::from_parts(77, 0),
        Err(DataTypeError::PrecisionOutOfRange {
            precision: 77,
            max: 76
        })
    );
    // Validation also applies on the way in from Arrow.
    assert!(Decimal128::from_arrow(&ArrowDataType::Decimal128(39, 0)).is_err());
}

#[test]
fn fixed_size_binary_roundtrips() {
    for size in [0, 1, 16, i32::MAX] {
        let binary = FixedSizeBinary::from_parts(size).unwrap();
        assert_roundtrip(binary, ArrowDataType::FixedSizeBinary(size));
    }
    assert_eq!(
        FixedSizeBinary::from_parts(-1),
        Err(DataTypeError::NegativeFixedSize { size: -1 })
    );
    assert!(FixedSizeBinary::from_arrow(&ArrowDataType::FixedSizeBinary(-1)).is_err());
}

#[test]
fn timestamps_roundtrip_over_units_and_timezones() {
    // A concrete unit type round-trips natively and rejects other units.
    for timezone in [None, Some("UTC"), Some("+02:00")] {
        let timestamp = Timestamp::from_parts(Nanosecond, timezone.map(Into::into));
        let arrow =
            ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, timezone.map(Into::into));
        assert_roundtrip(timestamp, arrow);
    }
    assert!(matches!(
        Timestamp::<Nanosecond>::from_arrow(&ArrowDataType::Timestamp(
            arrow_schema::TimeUnit::Second,
            None
        )),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));

    // The erased unit covers every native unit.
    for unit in TIME_UNITS {
        let timestamp = Timestamp::from_parts(AnyTimeUnit::from(unit), Some("UTC".into()));
        let arrow = ArrowDataType::Timestamp(unit.to_arrow().unwrap(), Some("UTC".into()));
        assert_roundtrip(timestamp, arrow);
    }
}

#[test]
fn extended_unit_timestamps_roundtrip_through_fields() {
    // Arrow lacks these units, so the type anchors on Int64 plus ygg.*
    // metadata carried by the field.
    let minutes = TypedField::from_parts(
        "logged_at",
        Timestamp::from_parts(Minute, Some("UTC".into())),
        true,
        Default::default(),
    );
    let arrow = minutes.to_arrow();
    assert_eq!(arrow.data_type(), &ArrowDataType::Int64);
    assert_eq!(arrow.metadata().get(metadata::TYPE).unwrap(), "timestamp");
    assert_eq!(arrow.metadata().get(metadata::TIME_UNIT).unwrap(), "min");
    assert_eq!(arrow.metadata().get(metadata::TIMEZONE).unwrap(), "UTC");
    assert_eq!(TypedField::from_arrow(&arrow), Ok(minutes));

    // Timezone-less, user metadata preserved, and the erased type restores
    // the same value.
    let user_metadata = [("origin".to_string(), "cron".to_string())]
        .into_iter()
        .collect();
    let years = TypedField::from_parts(
        "vintage",
        AnyDataType::from(Timestamp::from_parts(Year, None)),
        false,
        user_metadata,
    );
    let arrow = years.to_arrow();
    assert_eq!(arrow.data_type(), &ArrowDataType::Int64);
    assert!(!arrow.metadata().contains_key(metadata::TIMEZONE));
    let decoded = TypedField::<AnyDataType>::from_arrow(&arrow).unwrap();
    assert_eq!(decoded, years);
    assert_eq!(decoded.metadata().get("origin").unwrap(), "cron");

    // Without its metadata, a bare Int64 is never a timestamp.
    assert!(matches!(
        Timestamp::<Minute>::from_arrow(&ArrowDataType::Int64),
        Err(DataTypeError::MissingMetadata { .. })
    ));
}

#[test]
fn unknown_ygg_metadata_is_rejected() {
    let plain = arrow_schema::Field::new("id", ArrowDataType::Int32, false).with_metadata(
        [("ygg.mystery".to_string(), "1".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(matches!(
        TypedField::<Int32>::from_arrow(&plain),
        Err(yggdryl_schema::FieldError::DataType(
            DataTypeError::UnknownMetadata { .. }
        ))
    ));

    let bad_unit = arrow_schema::Field::new("t", ArrowDataType::Int64, false).with_metadata(
        [
            (metadata::TYPE.to_string(), "timestamp".to_string()),
            (metadata::TIME_UNIT.to_string(), "flarg".to_string()),
        ]
        .into_iter()
        .collect(),
    );
    assert!(matches!(
        TypedField::<AnyDataType>::from_arrow(&bad_unit),
        Err(yggdryl_schema::FieldError::DataType(
            DataTypeError::InvalidMetadata { .. }
        ))
    ));

    let bad_type = arrow_schema::Field::new("t", ArrowDataType::Int64, false).with_metadata(
        [(metadata::TYPE.to_string(), "wormhole".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(matches!(
        TypedField::<AnyDataType>::from_arrow(&bad_type),
        Err(yggdryl_schema::FieldError::DataType(
            DataTypeError::InvalidMetadata { .. }
        ))
    ));
}

#[test]
fn times_roundtrip_and_validate_units() {
    for unit in [TimeUnitId::Second, TimeUnitId::Millisecond] {
        let time = Time32::from_parts(unit).unwrap();
        assert_roundtrip(time, ArrowDataType::Time32(unit.to_arrow().unwrap()));
        assert!(Time64::from_parts(unit).is_err());
    }
    for unit in [TimeUnitId::Microsecond, TimeUnitId::Nanosecond] {
        let time = Time64::from_parts(unit).unwrap();
        assert_roundtrip(time, ArrowDataType::Time64(unit.to_arrow().unwrap()));
        assert!(Time32::from_parts(unit).is_err());
    }
    // An out-of-range unit is rejected on the way in from Arrow too.
    assert_eq!(
        Time32::from_arrow(&ArrowDataType::Time32(arrow_schema::TimeUnit::Nanosecond)),
        Err(DataTypeError::TimeUnitMismatch {
            expected: "s or ms",
            actual: TimeUnitId::Nanosecond
        })
    );
}

#[test]
fn durations_roundtrip_over_units() {
    for unit in TIME_UNITS {
        let duration = Duration::from_parts(unit).unwrap();
        assert_roundtrip(duration, ArrowDataType::Duration(unit.to_arrow().unwrap()));
    }
    // Arrow durations stop at the sub-second units.
    assert!(matches!(
        Duration::from_parts(TimeUnitId::Minute),
        Err(DataTypeError::TimeUnitMismatch { .. })
    ));
}

#[test]
fn lists_roundtrip_including_nesting() {
    let item = Arc::new(TypedField::from_parts(
        "item",
        Int32,
        true,
        Default::default(),
    ));
    assert_roundtrip(
        List::from_parts(item.clone()),
        ArrowDataType::List(Arc::new(item.to_arrow())),
    );
    assert_roundtrip(
        LargeList::from_parts(item.clone()),
        ArrowDataType::LargeList(Arc::new(item.to_arrow())),
    );

    let inner = List::from_parts(item);
    let outer = List::from_parts(Arc::new(TypedField::from_parts(
        "rows",
        inner,
        false,
        Default::default(),
    )));
    assert_eq!(List::from_arrow(&outer.to_arrow()), Ok(outer));

    // A list whose child is the wrong type is rejected.
    let utf8_child = Arc::new(arrow_schema::Field::new("item", ArrowDataType::Utf8, true));
    assert!(List::<Int32>::from_arrow(&ArrowDataType::List(utf8_child)).is_err());
}

#[test]
fn fields_roundtrip_with_metadata() {
    let metadata = [("origin".to_string(), "sensor-7".to_string())]
        .into_iter()
        .collect();
    let field = TypedField::from_parts("reading", Float64, true, metadata);
    let arrow = field.to_arrow();
    assert_eq!(arrow.name(), "reading");
    assert!(arrow.is_nullable());
    assert_eq!(arrow.metadata().get("origin").unwrap(), "sensor-7");
    assert_eq!(TypedField::from_arrow(&arrow), Ok(field));
}

fn person() -> Struct {
    Struct::from_parts(vec![
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
    ])
}

#[test]
fn structs_roundtrip_including_empty_and_nested() {
    let person = person();
    assert_eq!(Struct::from_arrow(&person.to_arrow()), Ok(person.clone()));
    assert_eq!(
        Struct::from_arrow(&Struct::from_parts(vec![]).to_arrow()),
        Ok(Struct::from_parts(vec![]))
    );

    // A struct of a struct round-trips too.
    let nested = Struct::from_parts(vec![Arc::new(TypedField::from_parts(
        "person",
        person.into(),
        true,
        Default::default(),
    ))]);
    assert_eq!(Struct::from_arrow(&nested.to_arrow()), Ok(nested));
    assert!(Struct::from_arrow(&ArrowDataType::Int32).is_err());
}

#[test]
fn maps_roundtrip_and_validate_entries() {
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person(),
        false,
        Default::default(),
    ));
    for sorted in [false, true] {
        let map = Map::from_parts(entries.clone(), sorted).unwrap();
        assert_eq!(Map::from_arrow(&map.to_arrow()), Ok(map));
    }

    // A nullable key or a wrong field count is rejected, from Arrow too.
    let nullable_key = Struct::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "key",
            Utf8.into(),
            true,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "value",
            Int32.into(),
            true,
            Default::default(),
        )),
    ]);
    let nullable_key = Arc::new(TypedField::from_parts(
        "entries",
        nullable_key,
        false,
        Default::default(),
    ));
    assert!(matches!(
        Map::from_parts(nullable_key, false),
        Err(DataTypeError::InvalidMapEntries { .. })
    ));
    let one_field = Struct::from_parts(vec![Arc::new(TypedField::from_parts(
        "key",
        Utf8.into(),
        false,
        Default::default(),
    ))]);
    let one_field = Arc::new(TypedField::from_parts(
        "entries",
        one_field,
        false,
        Default::default(),
    ));
    assert!(Map::from_parts(one_field, false).is_err());
}

#[test]
fn any_data_type_roundtrips_every_constructor() {
    let item = Arc::new(TypedField::from_parts(
        "item",
        AnyDataType::from(Int32),
        true,
        Default::default(),
    ));
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person(),
        false,
        Default::default(),
    ));
    let values: Vec<AnyDataType> = vec![
        Boolean.into(),
        Int8.into(),
        UInt64.into(),
        Float64.into(),
        Decimal128::from_parts(38, 2).unwrap().into(),
        Utf8.into(),
        FixedSizeBinary::from_parts(16).unwrap().into(),
        Date32.into(),
        Timestamp::from_parts(Nanosecond, Some("UTC".into())).into(),
        List::from_parts(item.clone()).into(),
        LargeList::from_parts(item).into(),
        person().into(),
        Map::from_parts(entries, true).unwrap().into(),
    ];
    for value in values {
        assert_eq!(AnyDataType::from_arrow(&value.to_arrow()), Ok(value));
    }

    // Unsupported Arrow types are rejected with a typed error.
    assert!(matches!(
        AnyDataType::from_arrow(&ArrowDataType::Float16),
        Err(DataTypeError::ArrowTypeMismatch { .. })
    ));
}
