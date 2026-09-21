//! `rust/src/serde.rs`: the stable structural document a value serializes as,
//! and what reading one back refuses.

mod value {

    use yggdryl::Geometry;
    use yggdryl::{Scalar, TimeUnit, Timezone, i256};

    /// One value of every kind, in the order [`Scalar`]'s total ordering puts them.
    ///
    /// Every test that has to be exhaustive over the value model reads this list,
    /// so a new variant is added here once rather than forgotten in three places.
    fn one_of_every_kind() -> Vec<Scalar> {
        vec![
            Scalar::Null,
            Scalar::from(true),
            Scalar::from(i128::MIN),
            Scalar::from(-8_i8),
            Scalar::from(-4_i16),
            Scalar::from(-2_i32),
            Scalar::from(-1_i64),
            Scalar::from(1_u8),
            Scalar::from(2_u16),
            Scalar::from(3_u32),
            Scalar::from(u64::MAX),
            Scalar::from(u128::MAX),
            Scalar::from(half::f16::from_f32(1.0)),
            Scalar::from(1.25_f32),
            Scalar::from(1.5),
            Scalar::d128(-1_050, 2),
            Scalar::d256(i256::from_i128(1_050), 2),
            Scalar::from("AAPL"),
            Scalar::from(b"\x00\xff".as_slice()),
            Scalar::date32(19_723),
            Scalar::date64(1_704_067_200_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(2_000_000, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64_in(1_700_000_000, TimeUnit::Second, "Europe/Paris").unwrap(),
            Scalar::duration32(1, TimeUnit::Second).unwrap(),
            Scalar::duration64(2, TimeUnit::Second).unwrap(),
            Scalar::from_sequence([Scalar::Null]),
            Scalar::from_mapping([(Scalar::from("k"), Scalar::Null)]).unwrap(),
            Scalar::from_struct([("k", Scalar::Null)]).unwrap(),
            Scalar::Geometry(
                Geometry::new([
                    1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ])
                .unwrap(),
            ),
        ]
    }

    #[test]
    fn structural_serde_reads_back_every_variant() {
        // The hand-written `Deserialize` mirrors `Scalar` variant for variant, and a
        // variant missing from the mirror is not a compile error - it is data serde
        // silently refuses to read. This is the check that makes it loud.
        let naive = Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap();
        for value in one_of_every_kind().into_iter().chain([naive]) {
            let encoded = serde_json::to_vec(&value).unwrap();
            let decoded = serde_json::from_slice::<Scalar>(&encoded).unwrap();
            assert_eq!(decoded, value, "{} did not survive serde", value.kind());
            assert_eq!(decoded.kind(), value.kind());
        }
    }

    #[test]
    fn structural_serde_preserves_all_float_values() {
        for value in [0.0, -0.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let value = Scalar::from(value);
            let encoded = serde_json::to_vec(&value).unwrap();
            assert_eq!(serde_json::from_slice::<Scalar>(&encoded).unwrap(), value);
        }
    }

    #[test]
    fn structural_serde_rejects_duplicate_mapping_keys() {
        let encoded = br#"{
        "type":"mapping",
        "value":[
            [{"type":"string","value":"a"},{"type":"null"}],
            [{"type":"string","value":"a"},{"type":"bool","value":true}]
        ]
    }"#;
        assert!(serde_json::from_slice::<Scalar>(encoded).is_err());
    }
}

mod datatypes {
    use yggdryl::{DataType, TimeUnit, Timezone};

    #[test]
    fn structural_json_uses_tagged_objects_and_rejects_bad_shapes() {
        let dtype = DataType::from_str("struct<id:bigint,tags:array<string>>").unwrap();
        let dtype_json = dtype.clone().into_json().unwrap();
        let dtype_value: serde_json::Value = serde_json::from_str(&dtype_json).unwrap();
        assert_eq!(dtype_value["type"], "struct");
        assert!(dtype_value["fields"].is_array());
        assert_eq!(dtype_value["fields"][0]["dtype"]["type"], "int64");
        let serde_json = serde_json::to_string(&dtype).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&serde_json).unwrap(),
            dtype
        );

        for malformed_or_invalid in [
            "{",
            r#"{"type":"not_a_datatype"}"#,
            r#"{"type":"int64","unknown":true}"#,
            r#"{"type":"datetime64","unit":"year_month"}"#,
            r#"{"type":"interval","unit":"second"}"#,
            r#"{"type":"decimal128","precision":0,"scale":0}"#,
            r#"{"type":"fixed_size_binary","width":16}"#,
            r#"{"type":"binary","layout":"fixed_size_binary"}"#,
            r#"{"type":"binary","max":0}"#,
            r#"{"type":"utf8"}"#,
        ] {
            assert!(
                DataType::from_json(malformed_or_invalid).is_err(),
                "{malformed_or_invalid}"
            );
        }
    }

    #[test]
    fn datetime64_json_uses_the_canonical_tag_and_explicit_timezone_model() {
        let naive = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap();
        let naive_json = naive.clone().into_json().unwrap();
        assert_eq!(naive_json, r#"{"type":"datetime64","unit":"microsecond"}"#);
        assert_eq!(DataType::from_json(&naive_json).unwrap(), naive);

        let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let utc_json = utc.clone().into_json().unwrap();
        assert_eq!(
            utc_json,
            r#"{"type":"datetime64","unit":"nanosecond","timezone":"UTC"}"#
        );
        assert_eq!(DataType::from_json(&utc_json).unwrap(), utc);

        assert!(DataType::from_json(r#"{"type":"timestamp","unit":"microsecond"}"#).is_err());
    }

    #[test]
    fn an_unsigned_width_serializes_as_the_name_every_other_door_spells() {
        // `rename_all = "snake_case"` turned `UInt8` into `u_int8`, which nothing
        // else in the crate says: the identifier, the text parser, `Display` and
        // both bindings all spell `uint8`. This is the `i_o_kind` defect one enum
        // over, and the same fix - name the four explicitly.
        for (dtype, spelled) in [
            (DataType::UInt8, "uint8"),
            (DataType::UInt16, "uint16"),
            (DataType::UInt32, "uint32"),
            (DataType::UInt64, "uint64"),
        ] {
            let document = dtype.clone().into_json().unwrap();
            assert!(
                document.contains(&format!("\"{spelled}\"")),
                "{spelled}: {document}"
            );
            assert!(!document.contains("u_int"), "{document}");
            assert_eq!(DataType::from_json(&document).unwrap(), dtype);
            assert_eq!(dtype.id().as_str(), spelled);
        }

        // A document written before the rename still reads, so nothing stored
        // stops loading.
        assert_eq!(
            DataType::from_json(r#"{"type":"u_int32"}"#).unwrap(),
            DataType::UInt32
        );
    }
}

mod families {

    use yggdryl::Field;
    use yggdryl::{
        BytesType, DataType, DecimalType, DictionaryType, RunEndEncodedType, StringType, TimeType,
        TimeUnit,
    };

    #[test]
    fn structural_json_rejects_malformed_and_duplicate_values() {
        assert_eq!(DataType::Int64.into_json().unwrap(), r#"{"type":"int64"}"#);
        assert_eq!(
            DataType::decimal128(38, 6).unwrap().into_json().unwrap(),
            r#"{"type":"decimal128","precision":38,"scale":6}"#
        );

        let field = |name: &str, dtype: serde_json::Value, nullable: bool| {
            serde_json::json!({
                "name": name,
                "dtype": dtype,
                "nullable": nullable,
                "metadata": {}
            })
        };

        let malformed = [
            serde_json::json!({"type": "time32", "unit": "nanosecond"}),
            serde_json::json!({"type": "binary", "layout": "fixed_size_binary", "fixed": 0}),
            serde_json::json!({"type": "decimal32", "precision": 10, "scale": 0}),
            serde_json::json!({
                "type": "struct",
                "fields": [
                    field("same", serde_json::json!({"type": "int32"}), false),
                    field("same", serde_json::json!({"type": "string"}), true)
                ]
            }),
            serde_json::json!({
                "type": "union",
                "mode": "dense",
                "fields": [
                    {"type_id": 1, "field": field("one", serde_json::json!({"type": "int32"}), false)},
                    {"type_id": 1, "field": field("two", serde_json::json!({"type": "string"}), true)}
                ]
            }),
            serde_json::json!({
                "type": "dictionary",
                "key": {"type": "float64"},
                "value": {"type": "string"}
            }),
            serde_json::json!({
                "type": "map",
                "entries": field(
                    "entries",
                    serde_json::json!({
                        "type": "struct",
                        "fields": [
                            field("key", serde_json::json!({"type": "string"}), false),
                            field("value", serde_json::json!({"type": "int64"}), true)
                        ]
                    }),
                    true
                ),
                "keys_sorted": false
            }),
            serde_json::json!({
                "type": "run_end_encoded",
                "run_ends": field("run_ends", serde_json::json!({"type": "int32"}), true),
                "values": field("values", serde_json::json!({"type": "string"}), true)
            }),
            serde_json::json!({"type": "int64", "unexpected": true}),
        ];

        for value in malformed {
            assert!(
                serde_json::from_value::<DataType>(value.clone()).is_err(),
                "accepted malformed structural datatype: {value}"
            );
        }

        let duplicate_metadata = r#"{
            "type":"list",
            "field":{
                "name":"item",
                "dtype":{"type":"string"},
                "nullable":true,
                "metadata":{"source":"one","source":"two"}
            }
        }"#;
        assert!(DataType::from_json(duplicate_metadata).is_err());
    }

    #[test]
    fn nested_serde_and_core_validators_keep_distinct_error_contracts() {
        let dictionary = serde_json::json!({
            "key": {"type": "float64"},
            "value": {"type": "string"}
        });
        assert_eq!(
            serde_json::from_value::<DictionaryType>(dictionary)
                .unwrap_err()
                .to_string(),
            "invalid Dictionary datatype: expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got float64"
        );
        assert_eq!(
            DataType::dictionary(DataType::Float64, DataType::utf8())
                .unwrap_err()
                .to_string(),
            "invalid Dictionary datatype: expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got float64"
        );

        let field = |name: &str, dtype: serde_json::Value, nullable: bool| {
            serde_json::json!({
                "name": name,
                "dtype": dtype,
                "nullable": nullable,
                "metadata": {}
            })
        };
        let run_end = serde_json::json!({
            "run_ends": field("run_ends", serde_json::json!({"type": "int32"}), true),
            "values": field("values", serde_json::json!({"type": "string"}), true)
        });
        assert_eq!(
            serde_json::from_value::<RunEndEncodedType>(run_end)
                .unwrap_err()
                .to_string(),
            "invalid RunEndEncoded datatype: expected a non-null run_ends field, got nullable field \"run_ends\""
        );
        assert_eq!(
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, true),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap_err()
            .to_string(),
            "invalid RunEndEncoded datatype: expected a non-null run_ends field, got nullable field \"run_ends\""
        );

        // The two run_ends rules are independent, so each reports the half that
        // actually failed rather than one fused sentence.
        assert_eq!(
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::utf8(), false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap_err()
            .to_string(),
            "invalid RunEndEncoded datatype: expected a run_ends datatype of int16, int32, or int64, got utf8"
        );
    }

    #[test]
    fn structural_serialization_rejects_public_enum_invalid_states() {
        let invalid = [
            DataType::Time(TimeType::Time32(TimeUnit::Nanosecond)),
            DataType::Bytes(BytesType::FixedBinary(0)),
            DataType::String(StringType::FixedUtf8String(0)),
            DataType::Decimal(DecimalType::Decimal128 {
                precision: 0,
                scale: 0,
            }),
        ];

        for value in invalid {
            assert!(
                serde_json::to_string(&value).is_err(),
                "serialized {value:?}"
            );
            assert!(value.clone().into_json().is_err(), "serialized {value:?}");
        }
    }
}

mod generic {

    use yggdryl::{DataType, Field};

    #[test]
    fn structural_json_uses_tagged_dtypes_and_rejects_bad_shapes() {
        let field = Field::new(
            "payload",
            DataType::from_str("struct<id:bigint,tags:array<string>>").unwrap(),
            false,
        );
        let field_json = field.into_json().unwrap();
        let field_value: serde_json::Value = serde_json::from_str(&field_json).unwrap();
        assert!(field_value.is_object());
        assert_eq!(field_value["dtype"]["type"], "struct");

        assert!(
            Field::from_json(
                r#"{"name":"id","dtype":{"type":"int64"},"nullable":false,"unknown":true}"#,
            )
            .is_err()
        );
        assert!(
            Field::from_json(
                r#"{"name":"id","dtype":{"type":"int64"},"nullable":false,"dictionary_id":7}"#,
            )
            .is_err()
        );
    }
}

mod schemas {
    use yggdryl::DateTimeType;
    use yggdryl::Scalar;
    use yggdryl::{DataType, Field, Metadata, PythonKind, PythonMetadata, StructType, TimeUnit};

    /// One representative field per shape the model can carry.
    fn shapes() -> Vec<Field> {
        let nested = StructType::from_fields([
            DataType::Int64.required_field("id"),
            StructType::from_fields([StructType::from_fields([
                DataType::utf8().nullable_field("leaf")
            ])
            .map(DataType::from)
            .unwrap()
            .nullable_field("inner")])
            .map(DataType::from)
            .unwrap()
            .required_field("middle"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("deep");

        let mut with_metadata =
            Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")]).unwrap();
        with_metadata.set_parquet_field_id(17);
        // A typed protocol vocabulary alongside the arbitrary key, so every format
        // is read back through the validator its `Deserialize` re-runs.
        with_metadata
            .as_python_mut()
            .set_class(
                &PythonMetadata::new("trading.book", "Book.Quote", PythonKind::Dataclass).unwrap(),
            )
            .unwrap();

        let mut dictionary = Field::new(
            "status",
            DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
            true,
        );
        dictionary.set_dictionary_options(42, true).unwrap();

        let partitioned = StructType::from_fields([
            DataType::utf8()
                .required_field("venue")
                .with_partition(true),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        vec![
            DataType::Int64.required_field("flat"),
            nested,
            DataType::list(
                StructType::from_fields([DataType::Int64.required_field("id")])
                    .map(DataType::from)
                    .unwrap()
                    .nullable_field("item"),
            )
            .nullable_field("rows"),
            DataType::map_of(DataType::utf8(), DataType::Int64, false)
                .unwrap()
                .nullable_field("counts"),
            DataType::union(
                [
                    (0_i8, DataType::Int64.nullable_field("number")),
                    (1_i8, DataType::utf8().nullable_field("text")),
                ],
                yggdryl::UnionMode::Dense,
            )
            .unwrap()
            .nullable_field("either"),
            dictionary,
            with_metadata,
            partitioned,
            DataType::decimal128(38, 6)
                .unwrap()
                .nullable_field("amount"),
            DataType::DateTime(DateTimeType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: "Europe/Paris".parse().unwrap(),
            })
            .nullable_field("at"),
            DataType::run_end_encoded(
                DataType::Int32.required_field("run_ends"),
                DataType::utf8().nullable_field("values"),
            )
            .unwrap()
            .nullable_field("runs"),
            DataType::fixed_binary(16).unwrap().nullable_field("uuid"),
            DataType::from_str("binary(64)")
                .unwrap()
                .nullable_field("blob"),
            DataType::large_binary().nullable_field("large"),
            DataType::from_str("utf8(32)")
                .unwrap()
                .nullable_field("note"),
            DataType::fixed_ascii(4).unwrap().required_field("ccy"),
            DataType::from_str("fixed_string(windows-1252,8)")
                .unwrap()
                .nullable_field("legacy"),
            DataType::from_str("large_utf8_view")
                .unwrap()
                .nullable_field("view"),
            DataType::Null.nullable_field("nothing"),
            DataType::variant().nullable_field("payload"),
            DataType::geometry(None).unwrap().nullable_field("shape"),
            DataType::geography(Some("EPSG:4326"), Some(yggdryl::EdgeAlgorithm::Vincenty))
                .unwrap()
                .required_field("region"),
        ]
    }

    #[test]
    fn every_shape_round_trips_through_the_value_conversion() {
        for field in shapes() {
            let restored = Field::from_value(field.clone().into_value()).expect("a field mapping");
            assert_eq!(restored, field, "{field}");
            assert_eq!(
                restored.as_metadata(),
                field.as_metadata(),
                "metadata survives {field}"
            );

            let dtype = field.dtype().clone();
            let restored =
                DataType::from_value(dtype.clone().into_value()).expect("a datatype mapping");
            assert_eq!(restored, dtype, "{dtype}");
        }
    }

    #[test]
    fn the_value_shape_matches_the_json_structure_exactly() {
        for field in shapes() {
            // The JSON path is expressed over this conversion, so the two must
            // produce the same bytes - this is the compatibility test that
            // pins `into_json` against drift.
            let direct = field.clone().into_json().expect("structural JSON");
            let through_value = String::from_utf8(
                yggdryl::json::into_bytes(&field.clone().into_value()).expect("a JSON dump"),
            )
            .expect("UTF-8");
            assert_eq!(direct, through_value, "{field}");

            let dtype = field.dtype();
            let direct = dtype.clone().into_json().expect("structural JSON");
            let through_value = String::from_utf8(
                yggdryl::json::into_bytes(&dtype.clone().into_value()).expect("a JSON dump"),
            )
            .expect("UTF-8");
            assert_eq!(direct, through_value, "{dtype}");
        }
    }

    #[test]
    fn a_malformed_mapping_is_refused_by_path() {
        let missing = Scalar::from_mapping([(Scalar::from("name"), Scalar::from("id"))]).unwrap();
        let refused = Field::from_value(missing).expect_err("no datatype");
        assert!(refused.to_string().contains("dtype"), "{refused}");

        let unknown =
            Scalar::from_mapping([(Scalar::from("type"), Scalar::from("quaternion"))]).unwrap();
        let refused = DataType::from_value(unknown).expect_err("no such datatype");
        assert!(refused.to_string().contains("quaternion"), "{refused}");

        let not_a_mapping = Scalar::from("int64");
        let refused = DataType::from_value(not_a_mapping).expect_err("not a mapping");
        assert!(
            refused.to_string().contains("datatype mapping"),
            "{refused}"
        );
    }

    #[test]
    fn metadata_entries_must_be_strings() {
        let field = Field::new("id", DataType::Int64, false);
        let mut value = field.into_value().as_mapping().unwrap().to_vec();
        value.pop();
        value.push((
            Scalar::from("metadata"),
            Scalar::from_mapping([(Scalar::from("count"), Scalar::from(7))]).unwrap(),
        ));
        let refused = Field::from_value(Scalar::from_mapping(value).unwrap())
            .expect_err("a non-string metadata value");
        assert!(refused.to_string().contains("count"), "{refused}");
    }

    #[test]
    fn the_trait_forms_sit_beside_the_inherent_ones() {
        let field = DataType::Int64.required_field("id");
        let value: Scalar = (&field).into();
        assert_eq!(Field::try_from(value).unwrap(), field);

        let dtype = DataType::utf8();
        let value: Scalar = (&dtype).into();
        assert_eq!(DataType::try_from(value).unwrap(), dtype);

        // Metadata keeps its own entries type; this is only the schema route.
        assert!(Metadata::new().is_empty());
    }

    /// One representative small field, for literal-text assertions.
    fn small() -> Field {
        StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    #[test]
    fn every_shape_round_trips_through_every_format() {
        for field in shapes() {
            assert_eq!(
                Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
                field
            );
            assert_eq!(
                Field::from_yaml(&field.clone().into_yaml().unwrap()).unwrap(),
                field
            );
            assert_eq!(
                Field::from_toml(&field.clone().into_toml().unwrap()).unwrap(),
                field
            );

            let dtype = field.dtype().clone();
            assert_eq!(
                DataType::from_json(&dtype.clone().into_json().unwrap()).unwrap(),
                dtype
            );
            assert_eq!(
                DataType::from_yaml(&dtype.clone().into_yaml().unwrap()).unwrap(),
                dtype
            );
            assert_eq!(
                DataType::from_toml(&dtype.clone().into_toml().unwrap()).unwrap(),
                dtype
            );
        }
    }

    #[test]
    fn every_string_and_byte_column_is_one_tag_with_its_parameters() {
        // One `"string"` tag carries the leaf and its number, each omitted when it
        // is the default - the leaf's name already says the charset; `"binary"`
        // the same over its six leaves.
        for (dtype, json) in [
            (DataType::utf8(), r#"{"type":"string"}"#),
            (
                DataType::from_str("utf8(32)").unwrap(),
                r#"{"type":"string","layout":"sized_utf8","max":32}"#,
            ),
            (
                DataType::fixed_ascii(4).unwrap(),
                r#"{"type":"string","layout":"fixed_ascii","fixed":4}"#,
            ),
            (DataType::ascii(), r#"{"type":"string","layout":"ascii"}"#),
            (
                DataType::large_utf8(),
                r#"{"type":"string","layout":"large_utf8"}"#,
            ),
            (
                DataType::from_str("string(windows-1252)").unwrap(),
                r#"{"type":"string","layout":"cp1252"}"#,
            ),
            (DataType::binary(), r#"{"type":"binary"}"#),
            (
                DataType::from_str("binary(16)").unwrap(),
                r#"{"type":"binary","layout":"sized_binary","max":16}"#,
            ),
            (
                DataType::fixed_binary(16).unwrap(),
                r#"{"type":"binary","layout":"fixed_binary","fixed":16}"#,
            ),
            (
                DataType::binary_view(),
                r#"{"type":"binary","layout":"binary_view"}"#,
            ),
        ] {
            assert_eq!(dtype.clone().into_json().unwrap(), json, "{dtype}");
            assert_eq!(DataType::from_json(json).unwrap(), dtype, "{json}");
        }

        // The retired tags name nothing.
        for retired in [
            r#"{"type":"utf8"}"#,
            r#"{"type":"large_utf8"}"#,
            r#"{"type":"utf8_view"}"#,
            r#"{"type":"ascii"}"#,
            r#"{"type":"fixed_ascii","width":4}"#,
            r#"{"type":"fixed_size_binary","width":16}"#,
            r#"{"type":"large_binary"}"#,
            r#"{"type":"binary_view"}"#,
        ] {
            assert!(DataType::from_json(retired).is_err(), "{retired}");
        }
        // A fixed layout needs its width, and one number has one reading.
        assert!(DataType::from_json(r#"{"type":"string","layout":"fixed_string"}"#).is_err());
        assert!(
            DataType::from_json(r#"{"type":"string","layout":"fixed_string","max":4}"#).is_err()
        );
        assert!(DataType::from_json(r#"{"type":"binary","fixed":4}"#).is_err());
    }

    #[test]
    fn natural_text_objects_feed_record_aware_structural_readers() {
        let field = small();
        let documents = [
            yggdryl::json::from_utf8(&field.clone().into_json().unwrap()).unwrap(),
            yggdryl::yaml::from_utf8(&field.clone().into_yaml().unwrap()).unwrap(),
            yggdryl::toml::from_utf8(&field.clone().into_toml().unwrap()).unwrap(),
        ];

        for document in documents {
            assert!(document.as_struct().is_some(), "{document:?}");
            let dtype = document.get_key_str("dtype").unwrap().clone();
            assert!(dtype.as_struct().is_some(), "{dtype:?}");
            assert_eq!(DataType::from_value(dtype).unwrap(), field.dtype().clone());
            assert_eq!(Field::from_value(document).unwrap(), field);
        }
    }

    #[test]
    fn the_three_formats_parse_back_to_equal_values() {
        // Cross-format agreement is by construction - one `Scalar` mapping feeds
        // all three - and this is the test that pins it.
        for field in shapes() {
            let from_json = Field::from_json(&field.clone().into_json().unwrap()).unwrap();
            let from_yaml = Field::from_yaml(&field.clone().into_yaml().unwrap()).unwrap();
            let from_toml = Field::from_toml(&field.clone().into_toml().unwrap()).unwrap();
            assert_eq!(from_json, from_yaml, "{field}");
            assert_eq!(from_yaml, from_toml, "{field}");
        }
    }

    #[test]
    fn a_small_dump_reads_the_way_the_docs_say() {
        // Literal text, so a formatting regression fails a test rather than
        // passing unnoticed.
        let field = small();

        assert_eq!(
            field.clone().into_json().unwrap(),
            r#"{"name":"row","dtype":{"type":"struct","fields":[{"name":"id","dtype":{"type":"int64"},"nullable":false,"metadata":{}}]},"nullable":false,"metadata":{}}"#
        );

        assert_eq!(
            field.clone().into_yaml().unwrap(),
            "\
name: row
dtype:
  type: struct
  fields:
    - name: id
      dtype:
        type: int64
      nullable: false
      metadata: {}
nullable: false
metadata: {}
"
        );

        assert_eq!(
            field.into_toml().unwrap(),
            "\
\"name\" = \"row\"
\"dtype\" = {\"type\" = \"struct\", \"fields\" = [{\"name\" = \"id\", \"dtype\" = {\"type\" = \"int64\"}, \"nullable\" = false, \"metadata\" = {}}]}
\"nullable\" = false
\"metadata\" = {}
"
        );
    }

    #[test]
    fn formatting_changes_bytes_never_meaning() {
        use yggdryl::text::{Formatting, Indent};

        let field = small();
        let settings = [
            Formatting::default(),
            Formatting::compact(),
            Formatting::indented(2),
            Formatting::indented(4),
            Formatting::default().with_indent(Indent::Tabs),
        ];

        for formatting in settings {
            for (dump, parse) in [
                (
                    Box::new(move |f: &Field| f.clone().into_json_with_formatting(formatting))
                        as Box<dyn Fn(&Field) -> yggdryl::Result<String>>,
                    Box::new(Field::from_json) as Box<dyn Fn(&str) -> yggdryl::Result<Field>>,
                ),
                (
                    Box::new(move |f: &Field| f.clone().into_yaml_with_formatting(formatting)),
                    Box::new(Field::from_yaml),
                ),
                (
                    Box::new(move |f: &Field| f.clone().into_toml_with_formatting(formatting)),
                    Box::new(Field::from_toml),
                ),
            ] {
                let text = dump(&field).unwrap();
                // Round-trip: parsing any formatting yields an equal value.
                let restored = parse(&text).unwrap();
                assert_eq!(restored, field, "{text}");
                // Idempotence: dumping again is byte-identical.
                assert_eq!(dump(&restored).unwrap(), text);
            }
        }
    }

    #[test]
    fn indentation_reads_literally_in_every_format() {
        use yggdryl::text::Formatting;

        let value = yggdryl::Scalar::from_mapping([
            (yggdryl::Scalar::from("id"), yggdryl::Scalar::from(1)),
            (
                yggdryl::Scalar::from("tags"),
                yggdryl::Scalar::from_sequence([yggdryl::Scalar::from("a")]),
            ),
        ])
        .unwrap();

        assert_eq!(
            yggdryl::json::into_bytes(&value).unwrap(),
            br#"{"id":1,"tags":["a"]}"#
        );
        assert_eq!(
            yggdryl::json::into_bytes_with_formatting(&value, Formatting::indented(2)).unwrap(),
            b"{\n  \"id\": 1,\n  \"tags\": [\n    \"a\"\n  ]\n}"
        );
        assert_eq!(
            yggdryl::json::into_bytes_with_formatting(&value, Formatting::indented(4)).unwrap(),
            b"{\n    \"id\": 1,\n    \"tags\": [\n        \"a\"\n    ]\n}"
        );

        assert_eq!(
            yggdryl::yaml::into_bytes(&value).unwrap(),
            b"id: 1\ntags:\n  - a\n"
        );
        assert_eq!(
            yggdryl::yaml::into_bytes_with_formatting(&value, Formatting::indented(4)).unwrap(),
            b"id: 1\ntags:\n    - a\n"
        );
        assert_eq!(
            yggdryl::yaml::into_bytes_with_formatting(&value, Formatting::compact()).unwrap(),
            b"{id: 1, tags: [a]}\n"
        );

        assert_eq!(
            yggdryl::toml::into_bytes(&value).unwrap(),
            b"\"id\" = 1\n\"tags\" = [\"a\"]\n"
        );
        assert_eq!(
            yggdryl::toml::into_bytes_with_formatting(&value, Formatting::indented(2)).unwrap(),
            b"\"id\" = 1\n\"tags\" = [\n  \"a\",\n]\n"
        );
    }

    #[test]
    fn depth_three_indents_one_level_per_level() {
        use yggdryl::text::Formatting;

        let deep = StructType::from_fields([StructType::from_fields([StructType::from_fields([
            DataType::Int64.required_field("leaf"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("inner")])
        .map(DataType::from)
        .unwrap()
        .required_field("middle")])
        .map(DataType::from)
        .unwrap()
        .required_field("outer");

        let yaml = deep
            .clone()
            .into_yaml_with_formatting(Formatting::indented(2))
            .unwrap();
        // One unit deeper per level, with no drift at depth.
        let columns: Vec<usize> = yaml
            .lines()
            .filter(|line| line.trim() == "type: struct")
            .map(|line| line.len() - line.trim_start().len())
            .collect();
        assert_eq!(columns.len(), 3, "{yaml}");
        assert_eq!(columns[1] - columns[0], columns[2] - columns[1], "{yaml}");
        assert!(columns[0] < columns[1], "{yaml}");

        let wide = deep
            .clone()
            .into_yaml_with_formatting(Formatting::indented(4))
            .unwrap();
        let wide_columns: Vec<usize> = wide
            .lines()
            .filter(|line| line.trim() == "type: struct")
            .map(|line| line.len() - line.trim_start().len())
            .collect();
        // A wider unit is strictly deeper and still drift-free. It is not simply
        // double: a level of this schema is one key indent plus one `- ` marker,
        // and the marker is always two columns because that is what YAML requires
        // for a sequence entry's continuation lines whatever the unit is.
        assert_eq!(wide_columns.len(), columns.len(), "{wide}");
        assert_eq!(
            wide_columns[1] - wide_columns[0],
            wide_columns[2] - wide_columns[1],
            "{wide}"
        );
        assert!(
            wide_columns[1] - wide_columns[0] > columns[1] - columns[0],
            "{wide}"
        );

        // JSON indents strictly by level, so the `fields` key at each depth sits
        // exactly one unit deeper than the one above it - no drift at depth.
        let json = deep
            .clone()
            .into_json_with_formatting(Formatting::indented(2))
            .unwrap();
        let json_columns: Vec<usize> = json
            .lines()
            .filter(|line| line.trim_start().starts_with("\"fields\""))
            .map(|line| line.len() - line.trim_start().len())
            .collect();
        assert_eq!(json_columns.len(), 3, "{json}");
        assert_eq!(
            json_columns[1] - json_columns[0],
            json_columns[2] - json_columns[1]
        );

        let wide_json = deep
            .into_json_with_formatting(Formatting::indented(4))
            .unwrap();
        let wide_json_columns: Vec<usize> = wide_json
            .lines()
            .filter(|line| line.trim_start().starts_with("\"fields\""))
            .map(|line| line.len() - line.trim_start().len())
            .collect();
        assert_eq!(
            wide_json_columns[1] - wide_json_columns[0],
            2 * (json_columns[1] - json_columns[0]),
            "{wide_json}"
        );
    }

    #[test]
    fn the_compact_form_is_unchanged_and_still_parses_back() {
        // The readable form is an addition; `Display` is untouched, because it
        // round-trips through the parsers and `__repr__` depends on it.
        for field in shapes() {
            let compact = field.to_string();
            assert!(!compact.contains('\n'), "{compact}");
            assert_eq!(Field::from_str(&compact).unwrap(), field);

            let dtype = field.dtype();
            let compact = dtype.to_string();
            assert!(!compact.contains('\n'), "{compact}");
            assert_eq!(DataType::from_str(&compact).unwrap(), *dtype);
        }
    }

    #[test]
    fn the_readable_form_indents_by_depth_and_omits_unset_attributes() {
        let mut field = StructType::from_fields([
            DataType::Int64.required_field("id"),
            StructType::from_fields([DataType::Float64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .nullable_field("line"),
            DataType::list(DataType::utf8().nullable_field("tag")).nullable_field("tags"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("order");
        field.insert_metadata("owner", "trading").unwrap();

        assert_eq!(
            format!("{field:#}"),
            "\
order: struct[3], required
  @owner = trading
  id: int64, required
  line: struct[1], nullable
    price: float64, required
  tags: list, nullable
    tag: utf8, nullable"
        );

        // `{:#}` and the named adapter are one implementation.
        assert_eq!(format!("{field:#}"), field.into_pretty_str());

        // Unset attributes are absent: no dictionary_id=0, no empty metadata blob.
        let plain = DataType::Int64.required_field("id");
        assert_eq!(format!("{plain:#}"), "id: int64, required");
        assert!(!format!("{plain:#}").contains("dictionary"));
        assert!(!format!("{plain:#}").contains("metadata"));

        // A set one is shown.
        let mut dictionary = DataType::dictionary(DataType::Int16, DataType::utf8())
            .unwrap()
            .nullable_field("status");
        dictionary.set_dictionary_options(42, true).unwrap();
        let readable = format!("{dictionary:#}");
        assert!(readable.contains("dictionary_id=42"), "{readable}");
        assert!(readable.contains("dictionary_is_ordered"), "{readable}");
    }

    #[test]
    fn the_readable_form_is_stable_across_runs() {
        // Nothing here iterates a hash map, so two renderings of one value - and
        // of two equal values built independently - agree exactly.
        let build = || {
            let mut field = StructType::from_fields([DataType::Int64.required_field("id")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            for (key, value) in [("z", "last"), ("a", "first"), ("m", "middle")] {
                field.insert_metadata(key, value).unwrap();
            }
            field
        };
        let first = build();
        let second = build();
        assert_eq!(format!("{first:#}"), format!("{second:#}"));
        assert_eq!(format!("{first:#}"), format!("{first:#}"));
        // Metadata renders as indented lines in key order, not one braced blob.
        assert!(format!("{first:#}").contains("\n  @a = first\n  @m = middle\n  @z = last\n"));
    }

    #[test]
    fn json_bytes_and_text_carry_the_same_nested_document() {
        // struct > list > struct > map, so the assertion is about nesting rather
        // than about a flat field.
        let inner = StructType::from_fields([
            DataType::utf8().required_field("sym"),
            DataType::decimal128(18, 4).unwrap().nullable_field("px"),
        ])
        .map(DataType::from)
        .unwrap();
        let row = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::list(inner.nullable_field("item")).nullable_field("levels"),
            DataType::map_of(DataType::utf8(), DataType::Int64, true)
                .unwrap()
                .nullable_field("tags"),
        ])
        .map(DataType::from)
        .unwrap();
        let mut field = row.required_field("row");
        field.set_comment("a deeply nested row").unwrap();

        // The two encodings are the same document, and each reads its own back.
        let text = field.clone().into_json().unwrap();
        let bytes = field.clone().into_json_bytes().unwrap();
        assert_eq!(bytes, text.as_bytes());
        assert_eq!(Field::from_json(&text).unwrap(), field);
        assert_eq!(Field::from_json_bytes(&bytes).unwrap(), field);

        // Nesting survives, rather than being flattened or stringified.
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["dtype"]["fields"][1]["dtype"]["type"], "list");
        assert_eq!(
            document["dtype"]["fields"][1]["dtype"]["field"]["dtype"]["fields"][0]["name"],
            "sym"
        );
        assert_eq!(document["dtype"]["fields"][2]["dtype"]["type"], "map");
        assert_eq!(document["metadata"]["comment"], "a deeply nested row");

        // A datatype answers the same pair.
        let dtype = field.dtype().clone();
        let dtype_bytes = dtype.clone().into_json_bytes().unwrap();
        assert_eq!(dtype_bytes, dtype.clone().into_json().unwrap().as_bytes());
        assert_eq!(DataType::from_json_bytes(&dtype_bytes).unwrap(), dtype);
    }

    #[test]
    fn every_format_round_trips_the_same_nested_field() {
        let field = StructType::from_fields([
            DataType::list(DataType::Int64.nullable_field("item")).nullable_field("levels"),
            StructType::from_fields([DataType::Boolean.required_field("ok")])
                .map(DataType::from)
                .unwrap()
                .required_field("flags"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // One structural model, three writers over it, so the three agree by
        // construction rather than by three sets of assertions.
        assert_eq!(
            Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
            field
        );
        assert_eq!(
            Field::from_yaml(&field.clone().into_yaml().unwrap()).unwrap(),
            field
        );
        assert_eq!(
            Field::from_toml(&field.clone().into_toml().unwrap()).unwrap(),
            field
        );
        assert_eq!(
            Field::from_value(field.clone().into_value()).unwrap(),
            field
        );
    }
}
