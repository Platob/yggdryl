use super::*;
use crate::types::Integer;

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields).unwrap().required_field("row")
}

#[test]
fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
    let schema = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ]);
    let record = Scalar::from_record([("id", Scalar::from(7))]).unwrap();

    schema.validate_value(&record).unwrap();
    assert_eq!(
        schema.canonicalize_value(record).unwrap(),
        Scalar::from_sequence([Scalar::from(7), Scalar::Null])
    );
}

#[test]
fn a_record_refuses_unknown_names() {
    let schema = root([DataType::Int64.required_field("id")]);
    let record =
        Scalar::from_record([("id", Scalar::from(7)), ("unknown", Scalar::from(1))]).unwrap();

    let validation = schema.validate_value(&record).unwrap_err().to_string();
    let canonical = schema.canonicalize_value(record).unwrap_err().to_string();
    assert!(validation.contains("unknown field"), "{validation}");
    assert!(canonical.contains("unknown field"), "{canonical}");
}

#[test]
fn integer_canonicalization_preserves_every_declared_width() {
    let schema = root([
        DataType::Int8.required_field("i8"),
        DataType::Int16.required_field("i16"),
        DataType::Int32.required_field("i32"),
        DataType::Int64.required_field("i64"),
        DataType::UInt8.required_field("u8"),
        DataType::UInt16.required_field("u16"),
        DataType::UInt32.required_field("u32"),
        DataType::UInt64.required_field("u64"),
    ]);
    let natural = Scalar::from_sequence([
        Scalar::from(-1),
        Scalar::from(2),
        Scalar::from(-3),
        Scalar::from(-4),
        Scalar::from(1),
        Scalar::from(2),
        Scalar::from(3),
        Scalar::from(4),
    ]);
    let canonical = schema.canonicalize_value(natural).unwrap();
    let values = canonical.as_sequence().unwrap();
    assert!(matches!(values[0], Scalar::Integer(Integer::I8(_))));
    assert!(matches!(values[1], Scalar::Integer(Integer::I16(_))));
    assert!(matches!(values[2], Scalar::Integer(Integer::I32(_))));
    assert!(matches!(values[3], Scalar::Integer(Integer::I64(_))));
    assert!(matches!(values[4], Scalar::Integer(Integer::U8(_))));
    assert!(matches!(values[5], Scalar::Integer(Integer::U16(_))));
    assert!(matches!(values[6], Scalar::Integer(Integer::U32(_))));
    assert!(matches!(values[7], Scalar::Integer(Integer::U64(_))));
}

#[test]
fn year_month_interval_keeps_its_signed_64_bit_component_spelling() {
    let schema = root([DataType::Interval(TimeUnit::YearMonth).required_field("months")]);

    assert_eq!(
        schema
            .canonicalize_value(Scalar::from_sequence([Scalar::from(18)]))
            .unwrap(),
        Scalar::from_sequence([Scalar::from(18)])
    );
}

#[test]
fn temporal_casts_preserve_family_and_timezone() {
    let schema = root([
        DataType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::UTC,
        }
        .required_field("at"),
        DataType::Time32(TimeUnit::Second).required_field("clock"),
        DataType::Duration32(TimeUnit::Millisecond).required_field("elapsed"),
    ]);
    let valid = Scalar::from_sequence([
        Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
        Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::duration64(3, TimeUnit::Second).unwrap(),
    ]);
    assert_eq!(
        schema.canonicalize_value(valid).unwrap(),
        Scalar::from_sequence([
            Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3_000, TimeUnit::Millisecond).unwrap(),
        ])
    );

    for invalid in [
        Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
        Scalar::from_sequence([
            Scalar::duration64(1, TimeUnit::Second).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
        Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
            Scalar::from("not a time"),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
    ] {
        assert!(schema.validate_value(&invalid).is_err());
        assert!(schema.canonicalize_value(invalid).is_err());
    }
}
