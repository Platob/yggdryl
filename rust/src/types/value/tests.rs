use super::*;

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields).unwrap().required_field("row")
}

#[test]
fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
    let schema = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ]);
    let record = Scalar::from_record([("id", Scalar::I8(7))]).unwrap();

    schema.validate_value(&record).unwrap();
    assert_eq!(
        schema.canonicalize_value(record).unwrap(),
        Scalar::from_sequence([Scalar::I64(7), Scalar::Null])
    );
}

#[test]
fn a_record_refuses_unknown_names() {
    let schema = root([DataType::Int64.required_field("id")]);
    let record =
        Scalar::from_record([("id", Scalar::I64(7)), ("unknown", Scalar::I64(1))]).unwrap();

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
        Scalar::I64(-1),
        Scalar::U64(2),
        Scalar::I64(-3),
        Scalar::I8(-4),
        Scalar::U64(1),
        Scalar::U64(2),
        Scalar::U64(3),
        Scalar::U8(4),
    ]);
    let canonical = schema.canonicalize_value(natural).unwrap();
    let values = canonical.as_sequence().unwrap();
    assert!(matches!(values[0], Scalar::I8(-1)));
    assert!(matches!(values[1], Scalar::I16(2)));
    assert!(matches!(values[2], Scalar::I32(-3)));
    assert!(matches!(values[3], Scalar::I64(-4)));
    assert!(matches!(values[4], Scalar::U8(1)));
    assert!(matches!(values[5], Scalar::U16(2)));
    assert!(matches!(values[6], Scalar::U32(3)));
    assert!(matches!(values[7], Scalar::U64(4)));
}

#[test]
fn year_month_interval_keeps_its_signed_64_bit_component_spelling() {
    let schema = root([DataType::Interval(TimeUnit::YearMonth).required_field("months")]);

    assert_eq!(
        schema
            .canonicalize_value(Scalar::from_sequence([Scalar::I8(18)]))
            .unwrap(),
        Scalar::from_sequence([Scalar::I64(18)])
    );
}

#[test]
fn temporal_casts_preserve_family_and_timezone() {
    let schema = root([
        DataType::Timestamp(TimeUnit::Millisecond, Some(Timezone::UTC)).required_field("at"),
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
            Scalar::Time32(2, TimeUnit::Second, Timezone::UTC),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
    ] {
        assert!(schema.validate_value(&invalid).is_err());
        assert!(schema.canonicalize_value(invalid).is_err());
    }
}
