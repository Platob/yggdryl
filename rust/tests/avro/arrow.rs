//! `rust/src/avro/arrow.rs`: the schema projection no caller can reach.
//!
//! `schema_json_from_field` is the crate-private rendering of a `Field` as an
//! Avro schema document, which every container write goes through. What a
//! caller can observe is beside it here and in the other files under
//! `rust/tests/avro/`.

#[test]
fn a_dictionary_column_writes_as_the_values_it_encodes() {
    use std::sync::Arc;

    use arrow_array::cast::AsArray;
    use arrow_array::types::Int32Type;
    use arrow_array::{Array, DictionaryArray, RecordBatch};
    use yggdryl::holder::Buffer;
    use yggdryl::{IOMedia, Url};

    // Avro has no dictionary encoding, and a dictionary is only an encoding
    // of its values: the container spells the values, and reads them back.
    let venues: DictionaryArray<Int32Type> = ["XNAS", "XNYS", "XNAS"].into_iter().collect();
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "venue",
        venues.data_type().clone(),
        false,
    )]));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(venues)]).unwrap();
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///t.avro").unwrap().media_type());
    let options = handle.record_options().unwrap();
    handle
        .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &options)
        .unwrap();

    let back: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let read = back[0].column(0).as_string::<i32>();
    assert_eq!(
        read.iter().collect::<Vec<_>>(),
        [Some("XNAS"), Some("XNYS"), Some("XNAS")]
    );
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::DataType;
    use yggdryl::StructType;
    use yggdryl::internals::avro_arrow::schema_json_from_field;

    #[test]
    fn a_root_union_reads_as_the_value_of_a_required_row() {
        use yggdryl::avro::Schema;
        use yggdryl::internals::avro_arrow::field_from_schema;

        // A root that is not a record is the `value` child of a required
        // `row`, and a union of null and a record is that record, nullable:
        // the root is not wrapped as a struct answers itself, because a
        // nullable root is what the record surface refuses.
        let record = r#"{"type":"record","name":"r","fields":[{"name":"id","type":"long"}]}"#;
        for (json, nullable) in [
            (format!(r#"["null",{record}]"#), true),
            (format!("[{record}]"), false),
        ] {
            let schema =
                Schema::from_json(&yggdryl::from_json_scalar(json.as_bytes()).unwrap()).unwrap();
            let root = field_from_schema(&schema, "row").unwrap();
            assert_eq!(root.name(), "row");
            assert!(!root.is_nullable());
            assert_eq!(root.field_len(), 1, "{json}");
            let value = root.get_field_at(0).unwrap();
            assert_eq!(value.name(), "value");
            assert_eq!(value.is_nullable(), nullable, "{json}");
            assert!(value.is_struct(), "{json}");
            assert_eq!(value.get_field_at(0).unwrap().name(), "id");
        }
    }

    #[test]
    fn an_ascii_column_is_an_avro_string() {
        let root = StructType::from_fields([
            DataType::fixed_ascii(4).unwrap().required_field("ccy"),
            DataType::fixed_ascii(16).unwrap().nullable_field("code"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let schema = schema_json_from_field(&root).unwrap();
        let fields = schema
            .get_key_str("fields")
            .and_then(yggdryl::Scalar::as_sequence)
            .unwrap();
        assert_eq!(
            fields[0]
                .get_key_str("type")
                .and_then(yggdryl::Scalar::as_str),
            Some("string")
        );
        let optional = fields[1]
            .get_key_str("type")
            .and_then(yggdryl::Scalar::as_sequence)
            .unwrap();
        assert!(
            optional
                .iter()
                .any(|branch| branch.as_str() == Some("string")),
            "{optional:?}"
        );
    }

    #[test]
    fn a_string_in_another_charset_is_not_an_avro_string() {
        // Avro's string is UTF-8; bytes in another charset are not, so the
        // column is refused by name rather than written as mojibake.
        let latin = DataType::cp1252();
        let root = StructType::from_fields([latin.required_field("label")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let message = schema_json_from_field(&root).unwrap_err().to_string();
        assert!(
            message.contains("expected a datatype Avro can spell"),
            "{message}"
        );
        assert!(message.contains("cp1252"), "{message}");
    }

    #[test]
    fn a_state_column_is_the_avro_int_its_code_is() {
        // A state stores the `int32` code of its member, so Avro writes the
        // integer and a declared field reads it back as the member.
        let root = StructType::from_fields([DataType::State.required_field("state")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let schema = schema_json_from_field(&root).unwrap();
        let fields = schema
            .get_key_str("fields")
            .and_then(yggdryl::Scalar::as_sequence)
            .unwrap();
        assert_eq!(
            fields[0]
                .get_key_str("type")
                .and_then(yggdryl::Scalar::as_str),
            Some("int")
        );
    }

    #[test]
    fn a_code_column_is_an_avro_string_and_a_code_key_is_spellable() {
        // Avro has no fixed-width text, so a code spells `string` with no
        // logical type - the contrast with a UUID, which annotates `uuid`.
        // Every registered code, read from the one listing: this used to name
        // four of them, and `side` and `timeinforce` were refused as
        // unspellable by a column spelling that had drifted
        // behind the family.
        let mut fields: Vec<_> = DataType::CODES
            .iter()
            .map(|(name, dtype, _)| dtype.clone().required_field(*name))
            .collect();
        // A map key gate that nothing else in the tree exercises for a
        // non-Utf8 key.
        fields.push(
            DataType::map_of(DataType::Mic, DataType::Int64, true)
                .unwrap()
                .required_field("by_venue"),
        );
        let codes = DataType::CODES.len();
        let root = DataType::from(StructType::from_fields(fields).unwrap()).required_field("row");
        let schema = schema_json_from_field(&root).unwrap();
        let fields = schema
            .get_key_str("fields")
            .and_then(yggdryl::Scalar::as_sequence)
            .unwrap();

        for ((name, ..), field) in DataType::CODES.iter().zip(fields.iter()) {
            assert_eq!(
                field.get_key_str("type").and_then(yggdryl::Scalar::as_str),
                Some("string"),
                "{name}"
            );
        }
        assert_eq!(
            fields[codes]
                .get_key_str("type")
                .and_then(|value| value.get_key_str("type"))
                .and_then(|value| value.as_str().map(str::to_owned)),
            Some("map".to_owned())
        );
    }
}

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType};

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    mod logical {
        use yggdryl::TimeUnit;
        use yggdryl::avro;
        use yggdryl::{DataType, DataTypeId, Scalar, Timezone};

        /// Round-trip one value through a single-field record container.
        fn round_trip(field_type: &str, value: Scalar) -> Scalar {
            let schema = yggdryl::json::from_utf8(&format!(
                r#"{{"type":"record","name":"row","fields":[{{"name":"v","type":{field_type}}}]}}"#
            ))
            .unwrap();
            let row = Scalar::from_mapping([(Scalar::from("v"), value)]).unwrap();
            let mut handle = super::buffer();
            avro::write_container(&mut handle, &schema, &[], &[row]).unwrap();
            let container = avro::read_container(&handle).unwrap();
            container.rows[0].get_key_str("v").unwrap().clone()
        }

        #[test]
        fn dates_round_trip_as_calendar_dates() {
            // 2024-02-29, a leap day, and a pre-epoch date.
            assert_eq!(
                round_trip(
                    r#"{"type":"int","logicalType":"date"}"#,
                    Scalar::date32(19_782)
                ),
                Scalar::date32(19_782)
            );
            assert_eq!(
                round_trip(
                    r#"{"type":"int","logicalType":"date"}"#,
                    Scalar::date32(-3_652)
                ),
                Scalar::date32(-3_652)
            );
            // A bare integer still encodes; it decodes as the calendar value.
            assert_eq!(
                round_trip(r#"{"type":"int","logicalType":"date"}"#, Scalar::from(3)),
                Scalar::date32(3)
            );
        }

        #[test]
        fn times_round_trip_at_their_declared_unit() {
            assert_eq!(
                round_trip(
                    r#"{"type":"int","logicalType":"time-millis"}"#,
                    Scalar::time32(86_399_999, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
                ),
                Scalar::time32(86_399_999, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
            );
            assert_eq!(
                round_trip(
                    r#"{"type":"long","logicalType":"time-micros"}"#,
                    Scalar::time32(1_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
                ),
                Scalar::time64(1_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
                "a coarser unit converts losslessly"
            );
        }

        #[test]
        fn timestamps_are_utc_instants_and_local_timestamps_stay_naive() {
            assert_eq!(
                round_trip(
                    r#"{"type":"long","logicalType":"timestamp-micros"}"#,
                    Scalar::datetime64(-1_000_000, TimeUnit::Microsecond, Timezone::UTC).unwrap()
                ),
                Scalar::datetime64(-1_000_000, TimeUnit::Microsecond, Timezone::UTC).unwrap()
            );
            assert_eq!(
                round_trip(
                    r#"{"type":"long","logicalType":"timestamp-nanos"}"#,
                    Scalar::from(123)
                ),
                Scalar::datetime64(123, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
            );
            assert_eq!(
                round_trip(
                    r#"{"type":"long","logicalType":"local-timestamp-millis"}"#,
                    Scalar::datetime64(555, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
                ),
                Scalar::datetime64(555, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
            );
        }

        #[test]
        fn a_lossy_unit_conversion_is_refused_naming_both_units() {
            let schema = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"long","logicalType":"time-micros"}}
            ]}"#,
            )
            .unwrap();
            let row = Scalar::from_mapping([(
                Scalar::from("v"),
                Scalar::time64(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            )])
            .unwrap();
            let mut handle = super::buffer();
            let message = avro::write_container(&mut handle, &schema, &[], &[row])
                .unwrap_err()
                .to_string();
            assert!(message.contains("from ns to us"), "{message}");
        }

        #[test]
        fn decimals_keep_their_unscaled_integer_and_scale() {
            // Nine, eighteen, and beyond-eighteen digits, over bytes.
            for unscaled in [
                123_456_789_i128,
                123_456_789_012_345_678,
                1_234_567_890_123_456_789_012_345_678,
            ] {
                for sign in [1, -1] {
                    let value = Scalar::decimal128(unscaled * sign, 2);
                    assert_eq!(
                        round_trip(
                            r#"{"type":"bytes","logicalType":"decimal","precision":38,"scale":2}"#,
                            value.clone()
                        ),
                        value
                    );
                }
            }
            // Over fixed, sign-extended to the declared width.
            let value = Scalar::decimal128(-12_345, 2);
            assert_eq!(
                round_trip(
                    r#"{"type":"fixed","name":"amount","size":16,"logicalType":"decimal","precision":20,"scale":2}"#,
                    value.clone()
                ),
                value
            );
        }

        #[test]
        fn an_overflowing_decimal_is_refused_naming_the_precision() {
            let schema = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"row","fields":[
                {"name":"v","type":{"type":"bytes","logicalType":"decimal","precision":4,"scale":0}}
            ]}"#,
            )
            .unwrap();
            let row = Scalar::from_mapping([(Scalar::from("v"), Scalar::decimal128(123_456, 0))])
                .unwrap();
            let mut handle = super::buffer();
            let message = avro::write_container(&mut handle, &schema, &[], &[row])
                .unwrap_err()
                .to_string();
            assert!(message.contains("4 digits"), "{message}");
        }

        #[test]
        fn uuids_round_trip_in_both_encodings() {
            let expected = DataType::uuid()
                .scalar("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
                .unwrap();
            assert_eq!(
                round_trip(
                    r#"{"type":"string","logicalType":"uuid"}"#,
                    Scalar::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6")
                ),
                expected
            );
            // The fixed form accepts the canonical text and stores the bytes.
            let decoded = round_trip(
                r#"{"type":"fixed","name":"id","size":16,"logicalType":"uuid"}"#,
                Scalar::from("f81d4fae-7dec-11d0-a765-00a0c91e6bf6"),
            );
            assert_eq!(decoded.id(), DataTypeId::Uuid);
            assert_eq!(decoded, expected);
        }

        #[test]
        fn fixed_and_decimal_schemas_restore_the_exact_leaf() {
            let fixed = round_trip(
                r#"{"type":"fixed","name":"raw","size":3}"#,
                Scalar::from([1_u8, 2, 3].as_slice()),
            );
            assert_eq!(fixed.id(), DataTypeId::FixedBinary);
            assert_eq!(fixed.dtype().unwrap(), DataType::fixed_binary(3).unwrap());

            for (precision, expected) in [
                (9, DataTypeId::Decimal32),
                (18, DataTypeId::Decimal64),
                (38, DataTypeId::Decimal128),
            ] {
                let decoded = round_trip(
                    &format!(
                        r#"{{"type":"bytes","logicalType":"decimal","precision":{precision},"scale":2}}"#
                    ),
                    Scalar::decimal128(12_345, 2),
                );
                assert_eq!(decoded.id(), expected, "precision {precision}");
            }
        }

        #[test]
        fn durations_round_trip_as_exact_intervals() {
            let value = Scalar::Interval(
                yggdryl::Interval::new(1, 2, 3_000_000, yggdryl::TimeUnit::MonthDayNano).unwrap(),
            );
            assert_eq!(
                round_trip(
                    r#"{"type":"fixed","name":"span","size":12,"logicalType":"duration"}"#,
                    value.clone()
                ),
                value
            );
        }

        #[test]
        fn an_unknown_logical_type_degrades_to_the_underlying_type() {
            assert_eq!(
                round_trip(
                    r#"{"type":"long","logicalType":"nobody-knows-this"}"#,
                    Scalar::from(9)
                ),
                Scalar::from(9)
            );
            // Invalid decimal attributes degrade too, per the specification.
            assert_eq!(
                round_trip(
                    r#"{"type":"bytes","logicalType":"decimal","precision":2,"scale":9}"#,
                    Scalar::from(&[1_u8, 2][..])
                ),
                Scalar::from(&[1_u8, 2][..])
            );
        }
    }
}
