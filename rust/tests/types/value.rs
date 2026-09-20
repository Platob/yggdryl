//! The value a datatype accepts: canonicalization, readings, and absence.

use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};
use yggdryl::{DateTimeType, DurationType, TimeType};

#[test]
fn variant_digest_keeps_the_native_depth_budget() {
    let mut native = Scalar::from(7_i64);
    for _ in 1..DataType::PARSE_RECURSION_LIMIT {
        native = Scalar::from_sequence([native]);
    }
    let held = Scalar::Variant(native.into_variant().unwrap());
    assert_eq!(
        held.digest(yggdryl::DigestAlgorithm::Xxh3),
        native.digest(yggdryl::DigestAlgorithm::Xxh3)
    );
}

#[test]
fn variant_conversions_share_the_value_and_scalar_contract() {
    use yggdryl::{Int64, UInt8, Value};

    let scalar = Scalar::from(7_i64);
    let leaf = Int64::from_scalar(&scalar).unwrap();
    let encoded = leaf.into_variant().unwrap();
    assert_eq!(encoded, scalar.into_variant().unwrap());
    assert_eq!(Int64::from_variant(&encoded).unwrap(), *leaf);
    assert_eq!(Scalar::from_variant(&encoded).unwrap(), scalar);

    let unsigned = Scalar::from(7_u8).into_variant().unwrap();
    let refused = UInt8::from_variant(&unsigned).unwrap_err();
    assert!(matches!(refused, yggdryl::Error::InvalidRecord { .. }));
    let message = refused.to_string();
    assert!(
        message.contains("UInt8") && message.contains("int16"),
        "{message}"
    );
    assert_eq!(
        DataType::UInt8.decode_variant(&unsigned).unwrap(),
        Scalar::from(7_u8)
    );

    let malformed = yggdryl::Variant::new(vec![0x11, 0, 0], vec![5 << 2]).unwrap();
    assert!(matches!(
        Int64::from_variant(&malformed),
        Err(yggdryl::Error::Codec { .. })
    ));
}

#[test]
fn variant_casts_cross_the_encoding_boundary_once() {
    let encoded = DataType::Variant.cast_scalar(&Scalar::from(7_i32)).unwrap();
    let Scalar::Variant(variant) = &encoded else {
        panic!("a variant value, got {encoded:?}");
    };
    assert_eq!(variant.scalar().unwrap(), Scalar::from(7_i32));
    assert_eq!(
        DataType::Int64.cast_scalar(&encoded).unwrap(),
        Scalar::from(7_i64)
    );
    assert_eq!(
        DataType::utf8().cast_scalar(&encoded).unwrap(),
        Scalar::from("7")
    );

    let encoded_null = DataType::Variant.cast_scalar(&Scalar::Null).unwrap();
    let Scalar::Variant(variant_null) = &encoded_null else {
        panic!("an encoded variant null, got {encoded_null:?}");
    };
    assert_eq!(variant_null.scalar().unwrap(), Scalar::Null);
    assert_eq!(
        DataType::Int64.cast_scalar(&encoded_null).unwrap(),
        Scalar::Null
    );

    let malformed = Scalar::Variant(yggdryl::Variant::new(vec![0x11, 0, 0], vec![5 << 2]).unwrap());
    assert!(matches!(
        DataType::Int64.cast_scalar(&malformed),
        Err(yggdryl::Error::Codec { .. })
    ));
    assert_eq!(DataType::Int64.try_cast_scalar(&malformed), Scalar::Null);
}

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(fields).unwrap()).required_field("row")
}

#[test]
fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
    let schema = root([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ]);
    let record = Scalar::from_struct([("id", Scalar::from(7))]).unwrap();

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
        Scalar::from_struct([("id", Scalar::from(7)), ("unknown", Scalar::from(1))]).unwrap();

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
    assert!(matches!(values[0], Scalar::Int8(_)));
    assert!(matches!(values[1], Scalar::Int16(_)));
    assert!(matches!(values[2], Scalar::Int32(_)));
    assert!(matches!(values[3], Scalar::Int64(_)));
    assert!(matches!(values[4], Scalar::UInt8(_)));
    assert!(matches!(values[5], Scalar::UInt16(_)));
    assert!(matches!(values[6], Scalar::UInt32(_)));
    assert!(matches!(values[7], Scalar::UInt64(_)));
}

#[test]
fn year_month_interval_canonicalizes_to_the_exact_interval_leaf() {
    let schema = root([DataType::interval(TimeUnit::YearMonth)
        .unwrap()
        .required_field("months")]);

    let canonical = schema
        .canonicalize_value(Scalar::from_sequence([Scalar::from(18)]))
        .unwrap();
    let value = &canonical.as_sequence().unwrap()[0];
    assert!(matches!(
        value,
        Scalar::Interval(interval)
            if interval.months() == 18 && interval.unit() == TimeUnit::YearMonth
    ));
}

#[test]
fn temporal_casts_preserve_family_and_timezone() {
    let schema = root([
        DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::UTC,
        })
        .required_field("at"),
        DataType::Time(TimeType::Time32(TimeUnit::Second)).required_field("clock"),
        DataType::Duration(DurationType::Duration32(TimeUnit::Millisecond))
            .required_field("elapsed"),
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

/// The spellings a value takes on the way into a datatype, and the ones it
/// prints on the way out - the same readings a column takes and prints.
mod readings {
    use yggdryl::{DataType, Scalar};
    use yggdryl::{Map, Mapping};

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    #[test]
    fn text_reads_into_every_family_a_text_column_reads_into() {
        assert_eq!(DataType::Int64.scalar("42").unwrap(), Scalar::from(42_i64));
        assert_eq!(DataType::UInt8.scalar(" 7 ").unwrap(), Scalar::from(7_u8));
        assert_eq!(
            DataType::Float64.scalar("4.5").unwrap(),
            Scalar::from(4.5_f64)
        );
        assert_eq!(
            DataType::Boolean.scalar("TRUE").unwrap(),
            Scalar::from(true)
        );
        assert_eq!(
            dtype("decimal128(10, 2)").scalar("10.50").unwrap(),
            Scalar::d128(1_050, 2)
        );
        assert_eq!(
            DataType::date32().scalar("1970-01-02").unwrap(),
            Scalar::date32(1)
        );

        // A magnitude the declared width cannot hold is refused by the width,
        // not wrapped by the reading.
        assert!(DataType::Int8.scalar("200").is_err());
        // A digit the declared scale cannot hold is refused rather than rounded.
        let refused = dtype("decimal128(10, 2)")
            .scalar("1.005")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("fractional digits"), "{refused}");
    }

    #[test]
    fn every_spelling_of_one_number_reaches_a_decimal_column_at_its_scale() {
        let column = dtype("decimal128(12, 2)");
        let hundred = Scalar::d128(10_000, 2);

        // A whole number is a decimal of scale zero, so writing it into a
        // column of scale two is one hundred, not one: the coefficient is
        // restated, never taken as though it were already unscaled.
        assert_eq!(column.scalar(100_i64).unwrap(), hundred);
        assert_eq!(column.scalar(Scalar::d128(100, 0)).unwrap(), hundred);
        assert_eq!(column.scalar("100.00").unwrap(), hundred);
        assert_eq!(column.scalar(Scalar::from(100_u8)).unwrap(), hundred);

        // The same holds at every declared width, and through a Field.
        for spelling in ["decimal32(9, 2)", "decimal64(12, 2)", "decimal256(40, 2)"] {
            let column = dtype(spelling);
            assert_eq!(
                column
                    .clone()
                    .required_field("size")
                    .scalar(100_i64)
                    .unwrap(),
                column.scalar(Scalar::d128(100, 0)).unwrap(),
                "{spelling}"
            );
        }

        // A negative scale removes digits, and only exactly.
        let tens = dtype("decimal128(12, -1)");
        assert_eq!(tens.scalar(100_i64).unwrap(), Scalar::d128(10, -1));
        assert!(tens.scalar(105_i64).is_err());
    }

    #[test]
    fn every_value_with_a_spelling_prints_it_into_a_text_column() {
        assert_eq!(DataType::utf8().scalar(7_i64).unwrap(), Scalar::from("7"));
        assert_eq!(
            DataType::utf8().scalar(Scalar::date32(0)).unwrap(),
            Scalar::from("1970-01-01")
        );
        assert_eq!(
            DataType::utf8()
                .scalar(DataType::Currency.scalar("USD").unwrap())
                .unwrap(),
            Scalar::from("USD")
        );
        assert_eq!(
            DataType::utf8()
                .scalar(Scalar::from(b"AAPL".to_vec()))
                .unwrap(),
            Scalar::from("AAPL")
        );

        // A payload that was read and refused names the charset and the byte
        // it refused, rather than which kind arrived.
        let refused = DataType::utf8()
            .scalar(Scalar::from(vec![0xFF_u8]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("utf-8"), "{refused}");
        assert!(refused.contains("0xff"), "{refused}");
    }

    #[test]
    fn a_byte_column_stores_the_payload_a_value_spells() {
        assert_eq!(
            DataType::binary().scalar("hi").unwrap(),
            Scalar::from(b"hi".to_vec())
        );
        // A code spells its characters' bytes, which is the payload its text
        // column stores; so does any other text, and nothing more.
        let code = DataType::Side.scalar("BUY").unwrap();
        assert_eq!(
            DataType::binary().scalar(code).unwrap().as_bytes(),
            Some(b"BUY".as_slice())
        );
        let text = dtype("fixed_ascii(4)").scalar("US").unwrap();
        assert_eq!(
            DataType::binary().scalar(text).unwrap().as_bytes(),
            Some(b"US".as_slice())
        );
        let uuid = DataType::uuid()
            .scalar("00000000-0000-0000-0000-000000000001")
            .unwrap();
        assert_eq!(
            DataType::fixed_binary(16)
                .unwrap()
                .scalar(uuid)
                .unwrap()
                .as_bytes(),
            Some(
                [0_u8; 15]
                    .iter()
                    .copied()
                    .chain([1])
                    .collect::<Vec<_>>()
                    .as_slice()
            )
        );

        // The declared width is part of the layout on every path.
        let refused = DataType::fixed_binary(4)
            .unwrap()
            .scalar(Scalar::from(vec![1_u8, 2]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("exactly 4 bytes"), "{refused}");
    }

    #[test]
    fn a_record_is_the_entries_a_map_column_holds() {
        let map = dtype("map<utf8, int32>");
        let record = Scalar::from_struct([("a", Scalar::from(1_i32))]).unwrap();

        assert_eq!(
            map.scalar(record).unwrap(),
            Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i32))]).unwrap()
        );
    }

    #[test]
    fn a_map_carries_its_invariants_however_its_entries_were_built() {
        // `Map::new` takes already-unique entries on trust, so the value
        // contract is what refuses a map that is not a function.
        let duplicates = Scalar::Mapping(Mapping::Map(Map::new(vec![
            (Scalar::from("a"), Scalar::from(1_i32)),
            (Scalar::from("a"), Scalar::from(2_i32)),
        ])));
        let refused = dtype("map<utf8, int32>")
            .scalar(duplicates)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("collide"), "{refused}");

        // A declared ordering is checked whether or not anything was restated.
        let unsorted = Scalar::Mapping(Mapping::Map(Map::new(vec![
            (Scalar::from("b"), Scalar::from(1_i32)),
            (Scalar::from("a"), Scalar::from(2_i32)),
        ])));
        let sorted = DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap();
        let refused = sorted.scalar(unsorted.clone()).unwrap_err().to_string();
        assert!(refused.contains("not sorted"), "{refused}");
        DataType::map_of(DataType::utf8(), DataType::Int32, false)
            .unwrap()
            .scalar(unsorted)
            .unwrap();
    }

    #[test]
    fn a_union_type_id_has_one_canonical_representation() {
        let union = dtype("union<0: int32>");
        let canonical = Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(1_i32)]);

        // The id does not depend on whether the payload also needed restating.
        assert_eq!(
            union
                .scalar(Scalar::from_sequence([
                    Scalar::from(0_i8),
                    Scalar::from(1_i32)
                ]))
                .unwrap(),
            canonical
        );
        assert_eq!(union.scalar(canonical.clone()).unwrap(), canonical);
    }
}

/// Absence is a value only where the layout stores it beside the values.
mod absence {
    use yggdryl::{DataType, Field, Scalar};

    #[test]
    fn a_union_and_a_run_end_spell_absence_through_a_child() {
        // Both layouts carry absence inside a child rather than beside the
        // values, so a bare null is not a value either of them holds - the
        // same rule the nested walk has always applied to a child.
        let union: DataType = "union<0: int32>".parse().unwrap();
        let refused = union.scalar(Scalar::Null).unwrap_err().to_string();
        assert!(refused.contains("[type_id, payload]"), "{refused}");
        assert!(
            Field::new("u", union.clone(), true)
                .scalar(Scalar::Null)
                .is_err()
        );
        // The pair that does spell it is accepted.
        union
            .scalar(Scalar::from_sequence([Scalar::from(0_i64), Scalar::Null]))
            .unwrap();

        let required = DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Int32.required_field("values"),
        )
        .unwrap();
        let refused = required.scalar(Scalar::Null).unwrap_err().to_string();
        assert!(refused.contains("non-nullable"), "{refused}");
        // A run-end layout whose values child holds a null does hold one.
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Int32.nullable_field("values"),
        )
        .unwrap()
        .scalar(Scalar::Null)
        .unwrap();

        // Every other datatype still takes absence as a value of its own.
        assert_eq!(DataType::Int32.scalar(Scalar::Null).unwrap(), Scalar::Null);
    }
}
