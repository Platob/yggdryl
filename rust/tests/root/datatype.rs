//! `rust/src/datatype.rs`.

mod arrow {
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone, UnionMode};

    fn assert_invalid(error: yggdryl::Error, expected_kind: &str, expected_reason: &str) {
        match error {
            yggdryl::Error::InvalidDataType { kind, reason } => {
                assert_eq!(kind, expected_kind);
                assert_eq!(reason, expected_reason);
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[test]
    fn borrowed_and_consuming_arrow_datatype_paths_are_lossless() {
        let dtype =
            DataType::from_str("struct<id:bigint,values:map<string,array<decimal(38,18)>>>")
                .unwrap();
        let borrowed = dtype.clone().into_arrow_datatype().unwrap();
        assert_eq!(DataType::from_arrow_datatype(&borrowed).unwrap(), dtype);
        let owned = dtype.clone().into_arrow_datatype().unwrap();
        assert_eq!(DataType::try_from(owned).unwrap(), dtype);
    }

    #[test]
    fn direct_arrow_values_round_trip_through_core() {
        let arrow = ArrowDataType::Struct(
            vec![
                Arc::new(ArrowField::new("id", ArrowDataType::Int64, false)),
                Arc::new(ArrowField::new("name", ArrowDataType::Utf8, true)),
            ]
            .into(),
        );
        let core = DataType::from_arrow_datatype(&arrow).unwrap();
        assert_eq!(core.clone().into_arrow_datatype().unwrap(), arrow);
        assert_eq!(core.get_field(0).unwrap().name(), "id");
        assert_eq!(
            core.get_field_by_path("name").unwrap().dtype(),
            &DataType::utf8()
        );
    }

    #[test]
    fn every_temporal_and_interval_unit_round_trips_through_all_core_formats() {
        let values = [
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
            DataType::DateTime64 {
                unit: TimeUnit::Millisecond,
                timezone: Timezone::UTC,
            },
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::from_str("Europe/Paris").unwrap(),
            },
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::NAIVE,
            },
            DataType::Time32(TimeUnit::Second),
            DataType::Time32(TimeUnit::Millisecond),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Time64(TimeUnit::Nanosecond),
            DataType::Duration64(TimeUnit::Second),
            DataType::Duration64(TimeUnit::Millisecond),
            DataType::Duration64(TimeUnit::Microsecond),
            DataType::Duration64(TimeUnit::Nanosecond),
            DataType::Interval(TimeUnit::YearMonth),
            DataType::Interval(TimeUnit::DayTime),
            DataType::Interval(TimeUnit::MonthDayNano),
        ];

        for value in values {
            value.validate().unwrap();
            let arrow = value.clone().into_arrow_datatype().unwrap();
            assert_eq!(DataType::from_arrow_datatype(&arrow).unwrap(), value);
            assert_eq!(DataType::try_from(arrow.clone()).unwrap(), value);
            assert_eq!(value.clone().into_arrow_datatype().unwrap(), arrow);

            let displayed = value.to_string();
            assert_eq!(DataType::from_str(&displayed).unwrap(), value);
            let json = value.clone().into_json().unwrap();
            assert_eq!(DataType::from_json(&json).unwrap(), value);
            assert_eq!(serde_json::from_str::<DataType>(&json).unwrap(), value);
        }
    }

    #[test]
    fn duration32_projects_to_arrow_and_imports_at_arrows_native_width() {
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            let narrow = DataType::duration32(unit).unwrap();
            let arrow = narrow.into_arrow_datatype().unwrap();
            assert_eq!(
                arrow,
                ArrowDataType::Duration(unit.into_arrow_time().unwrap())
            );
            assert_eq!(
                DataType::from_arrow_datatype(&arrow).unwrap(),
                DataType::duration64(unit).unwrap()
            );
        }
    }

    #[test]
    fn every_arrow_datatype_variant_round_trips_borrowed_owned_display_json_and_debug() {
        let item = || Field::new("item", DataType::utf8(), true);
        let entries = || {
            Field::new(
                "entries",
                StructType::from_fields([
                    Field::new("key", DataType::utf8(), false),
                    Field::new("value", DataType::Int64, true),
                ])
                .map(DataType::from)
                .unwrap(),
                false,
            )
        };
        let values = vec![
            DataType::Null,
            DataType::Boolean,
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::Float16,
            DataType::Float32,
            DataType::Float64,
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::from_str("Europe/Paris").unwrap(),
            },
            DataType::date32(),
            DataType::date64(),
            DataType::Time32(TimeUnit::Millisecond),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Duration64(TimeUnit::Nanosecond),
            DataType::Interval(TimeUnit::YearMonth),
            DataType::Interval(TimeUnit::DayTime),
            DataType::Interval(TimeUnit::MonthDayNano),
            DataType::binary(),
            DataType::fixed_binary(16).unwrap(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::serie(item()),
            DataType::serie_view(item()),
            DataType::fixed_size_serie(item(), 4).unwrap(),
            DataType::large_serie(item()),
            DataType::large_serie_view(item()),
            DataType::from(
                StructType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
            ),
            DataType::union(
                [
                    (0, Field::new("number", DataType::Int64, false)),
                    (7, Field::new("text", DataType::utf8(), true)),
                ],
                UnionMode::Dense,
            )
            .unwrap(),
            DataType::dictionary(DataType::UInt16, DataType::utf8()).unwrap(),
            DataType::decimal32(9, 2).unwrap(),
            DataType::decimal64(18, -2).unwrap(),
            DataType::decimal128(38, 18).unwrap(),
            DataType::decimal256(76, 20).unwrap(),
            DataType::map(entries(), true).unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
        ];

        for value in values {
            let borrowed = value.clone().into_arrow_datatype().unwrap();
            let ffi = value.clone().into_arrow_datatype_ffi().unwrap();
            assert_eq!(ArrowDataType::try_from(&ffi).unwrap(), borrowed);
            assert_eq!(DataType::from_arrow_datatype(&borrowed).unwrap(), value);
            assert_eq!(DataType::try_from(borrowed.clone()).unwrap(), value);
            assert_eq!(value.clone().into_arrow_datatype().unwrap(), borrowed);
            assert_eq!(DataType::from_str(&value.to_string()).unwrap(), value);
            assert_eq!(
                DataType::from_json(&value.clone().into_json().unwrap()).unwrap(),
                value
            );
            let debug = format!("{borrowed:?}");
            let parsed = DataType::from_str(&debug).unwrap_or_else(|error| {
                panic!("failed to parse Arrow debug form {debug}: {error}")
            });
            assert_eq!(parsed, value, "failed to parse Arrow debug form {debug}");
        }
    }

    /// Every datatype whose identity is an Arrow extension, and not the storage
    /// it is written over.
    fn extension_datatypes() -> Vec<DataType> {
        vec![
            DataType::ascii(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::Country,
            DataType::Ccy,
            DataType::Mic,
            DataType::Cfi,
            DataType::Uuid,
            DataType::Version,
            DataType::Variant,
            DataType::geometry(Some("EPSG:4326")).unwrap(),
            DataType::geography(Some("EPSG:4326"), None).unwrap(),
        ]
    }

    #[test]
    fn every_extension_datatype_survives_arrow_projection_in_every_shape() {
        for dtype in extension_datatypes() {
            // The identity is field metadata, so every shape that puts the type
            // under a field keeps it - including a dictionary encoding, whose
            // Arrow values are a bare datatype with nowhere of their own to
            // declare it.
            for held in [
                dtype.clone(),
                DataType::dictionary(DataType::Int32, dtype.clone()).unwrap(),
                DataType::serie(Field::new("item", dtype.clone(), true)),
                DataType::from(
                    StructType::from_fields([Field::new("child", dtype.clone(), true)]).unwrap(),
                ),
                DataType::run_end_encoded(
                    Field::new("run_ends", DataType::Int32, false),
                    Field::new("values", dtype.clone(), true),
                )
                .unwrap(),
            ] {
                let field = Field::new("f", held.clone(), true);
                let arrow = field.clone().into_arrow_field().unwrap();
                assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field, "{held}");

                // The C schema is a field node too, so it carries the same
                // identity, dictionary encoding included.
                let ffi = field.clone().into_arrow_field_ffi().unwrap();
                let imported = ArrowField::try_from(&ffi).unwrap();
                assert_eq!(
                    Field::from_arrow_field(&imported).unwrap().dtype(),
                    &held,
                    "{held}"
                );

                // A bare datatype has nowhere to carry it in Arrow, but its own C
                // schema does.
                let ffi = held.clone().into_arrow_datatype_ffi().unwrap();
                let imported = ArrowField::try_from(&ffi).unwrap();
                assert_eq!(
                    Field::from_arrow_field(&imported).unwrap().dtype(),
                    &held,
                    "{held}"
                );
            }

            // The documented exception: an Arrow datatype is storage, because it
            // has no metadata to name an extension with.
            let storage = dtype.clone().into_arrow_datatype().unwrap();
            assert_ne!(
                DataType::from_arrow_datatype(&storage).unwrap(),
                dtype,
                "{dtype}"
            );
        }
    }

    #[test]
    fn an_extension_schema_survives_an_ipc_round_trip() {
        let root = StructType::from_fields(
            extension_datatypes()
                .into_iter()
                .enumerate()
                .flat_map(|(index, dtype)| {
                    [
                        Field::new(format!("c{index}"), dtype.clone(), true),
                        Field::new(
                            format!("d{index}"),
                            DataType::dictionary(DataType::Int32, dtype).unwrap(),
                            true,
                        ),
                    ]
                })
                .collect::<Vec<_>>(),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let schema = root.clone().into_arrow_schema().unwrap();
        let mut buffer = Vec::new();
        let mut writer = arrow_ipc::writer::StreamWriter::try_new(&mut buffer, &schema).unwrap();
        writer.finish().unwrap();
        drop(writer);

        let reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(buffer), None).unwrap();
        let back = Field::from_arrow_schema("row", reader.schema().as_ref()).unwrap();

        // Datatypes, not whole fields: the IPC writer assigns each dictionary
        // column a transport dictionary ID, which is not part of the schema the
        // root declared.
        assert_eq!(back.field_len(), root.field_len());
        for (held, imported) in root.fields().iter().zip(back.fields().iter()) {
            assert_eq!(imported.name(), held.name());
            assert_eq!(imported.dtype(), held.dtype(), "{}", held.name());
        }
    }

    #[test]
    fn invalid_arrow_parameters_and_nested_shapes_fail_before_projection() {
        assert!(DataType::Time32(TimeUnit::Nanosecond).validate().is_err());
        assert!(DataType::Time64(TimeUnit::Second).validate().is_err());
        for invalid in [
            DataType::DateTime64 {
                unit: TimeUnit::YearMonth,
                timezone: Timezone::NAIVE,
            },
            DataType::Duration32(TimeUnit::DayTime),
            DataType::Duration64(TimeUnit::DayTime),
            DataType::Interval(TimeUnit::Second),
        ] {
            assert!(invalid.validate().is_err());
            assert!(invalid.clone().into_arrow_datatype().is_err());
            assert!(invalid.into_json().is_err());
        }
        assert!(DataType::fixed_binary(0).is_err());
        assert!(
            DataType::fixed_size_serie(Field::new("item", DataType::utf8(), true), -1).is_err()
        );
        assert!(DataType::decimal128(0, 0).is_err());
        assert!(DataType::decimal128(5, 6).is_err());
        assert!(DataType::dictionary(DataType::Float64, DataType::utf8()).is_err());
        assert!(
            DataType::map(
                Field::new(
                    "entries",
                    StructType::from_fields([
                        Field::new("key", DataType::utf8(), true),
                        Field::new("value", DataType::Int64, true),
                    ])
                    .map(DataType::from)
                    .unwrap(),
                    false,
                ),
                false,
            )
            .is_err()
        );
        assert!(
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::UInt32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .is_err()
        );

        let duplicate_arrow = ArrowDataType::Struct(
            vec![
                Arc::new(ArrowField::new("same", ArrowDataType::Int32, false)),
                Arc::new(ArrowField::new("same", ArrowDataType::Utf8, true)),
            ]
            .into(),
        );
        assert!(DataType::from_arrow_datatype(&duplicate_arrow).is_err());
        assert!(DataType::try_from(duplicate_arrow).is_err());
    }

    #[test]
    fn invariant_errors_match_across_construction_validation_and_arrow_projection() {
        let invalid_time = DataType::Time32(TimeUnit::Nanosecond);
        for error in [
            DataType::time32(TimeUnit::Nanosecond).unwrap_err(),
            invalid_time.validate().unwrap_err(),
            invalid_time.clone().into_arrow_datatype().unwrap_err(),
            invalid_time.into_arrow_datatype_ffi().unwrap_err(),
        ] {
            assert_invalid(error, "Time32", "unit must be second or millisecond");
        }

        // The constructor refuses a width of zero; the variant is public, so a
        // fixed layout can also be built with no width at all, and every door
        // past construction refuses that one alike.
        assert_invalid(
            DataType::fixed_binary(0).unwrap_err(),
            "bytes",
            "expected a width of at least one byte, got 0",
        );
        // A leaf carries its own count, so the state a caller can still build by
        // hand is a count of nothing.
        let invalid_binary = DataType::FixedBinary(0);
        for error in [
            invalid_binary.validate().unwrap_err(),
            invalid_binary.clone().into_arrow_datatype().unwrap_err(),
            invalid_binary.into_arrow_datatype_ffi().unwrap_err(),
        ] {
            assert_invalid(
                error,
                "bytes",
                "expected a width of at least one byte, got 0",
            );
        }

        let item = Field::new("item", DataType::utf8(), true);
        let invalid_serie = DataType::FixedSizeSerie(Arc::new(item.clone()), -1);
        for error in [
            DataType::fixed_size_serie(item, -1).unwrap_err(),
            invalid_serie.validate().unwrap_err(),
            invalid_serie.clone().into_arrow_datatype().unwrap_err(),
            invalid_serie.into_arrow_datatype_ffi().unwrap_err(),
        ] {
            assert_invalid(error, "FixedSizeSerie", "length must be non-negative: -1");
        }
    }

    #[test]
    fn every_extension_typed_datatype_keeps_its_identity_across_the_c_interface() {
        // `DataType::into_arrow_ffi` documents that it "keeps an extension
        // identity - a code, a UUID, a version, a variant, a geospatial parameter
        // set". It used to promise that against a hand-written list of datatypes,
        // which had drifted behind: `side`, `state`, `timeinforce` and every
        // `string(...)` fell through to the plain arm and crossed as anonymous
        // storage. The listing is now the one function that
        // answers which datatypes have an extension at all, so a datatype added
        // later cannot be added to one and forgotten in the other.
        let extension_typed = [
            DataType::ascii(),
            DataType::from_str("ascii(4)").unwrap(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::Country,
            DataType::Ccy,
            DataType::Mic,
            DataType::Cfi,
            DataType::Isin,
            DataType::Cusip,
            DataType::Sedol,
            DataType::Side,
            DataType::State,
            DataType::TimeInForce,
            DataType::Uuid,
            DataType::Version,
            DataType::url(),
            DataType::Variant,
            DataType::from_str("string(windows-1252)").unwrap(),
            DataType::from_str("fixed_string(windows-1252,8)").unwrap(),
            DataType::from_str("large_utf8_view").unwrap(),
        ];

        for dtype in extension_typed {
            let ffi = dtype.clone().into_arrow_datatype_ffi().unwrap();
            let arrow = ArrowField::try_from(&ffi)
                .unwrap_or_else(|error| panic!("{dtype} did not project a C schema: {error}"));
            let name = arrow
                .metadata()
                .get("ARROW:extension:name")
                .unwrap_or_else(|| {
                    panic!("{dtype} crossed the C Data Interface without its identity")
                });
            assert!(
                name.starts_with("yggdryl.") || name.starts_with("arrow.") || name.contains('.'),
                "{dtype} projected an unqualified extension name {name:?}"
            );
            // And what came back reads as the datatype that was sent.
            assert_eq!(
                Field::from_arrow_field(&arrow).unwrap().dtype(),
                &dtype,
                "{dtype} did not read back as itself"
            );
        }
    }
}

mod families {
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use std::collections::{BTreeSet, HashSet};
    use std::hash::Hash;
    use std::sync::Arc;
    use yggdryl::{
        BytesType, DataType, DataTypeKind, DataTypeValue, DateTimeType, DateType, DecimalType,
        DurationType, GeographyType, GeometryType, IntervalType, MapType, RunEndEncodedType,
        StringType, StructType, TimeType, TimeUnit, UnionMode,
    };
    use yggdryl::{Charset, Field, Timezone};

    #[test]
    fn datatype_payloads_round_trip_the_root_without_losing_parameters() {
        // An integer or a float carries nothing beyond its identifier, so it
        // has no payload: its family is the range its identifier is in.
        assert!(DataTypeKind::Integer.contains(DataType::UInt32.id()));
        assert!(DataType::UInt32.is_integer());
        assert!(DataTypeKind::Floating.contains(DataType::Float16.id()));
        assert!(!DataType::Float16.is_integer());

        let decimal = DataType::decimal128(20, 4).unwrap();
        let decimal_family = DecimalType::try_from(&decimal).unwrap();
        assert_eq!(DataType::from(decimal_family), decimal);

        // The five temporal families each hold their own payload; a unit or a
        // zone is a parameter of the leaf and survives the round trip.
        let datetime = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap();
        let datetime_family = DateTimeType::try_from(&datetime).unwrap();
        assert_eq!(datetime_family.unit(), TimeUnit::Microsecond);
        assert_eq!(DataType::from(datetime_family), datetime);

        let date = DateType::try_from(&DataType::date64()).unwrap();
        assert_eq!(date, DateType::Date64);
        assert_eq!(DataType::from(date), DataType::date64());

        let time = DataType::time64(TimeUnit::Nanosecond).unwrap();
        let time_family = TimeType::try_from(&time).unwrap();
        assert_eq!(time_family, TimeType::Time64(TimeUnit::Nanosecond));
        assert_eq!(DataType::from(time_family), time);

        let duration = DataType::duration32(TimeUnit::Second).unwrap();
        let duration_family = DurationType::try_from(&duration).unwrap();
        assert_eq!(duration_family, DurationType::Duration32(TimeUnit::Second));
        assert_eq!(DataType::from(duration_family), duration);

        let interval = DataType::interval(TimeUnit::DayTime).unwrap();
        let interval_family = IntervalType::try_from(&interval).unwrap();
        assert_eq!(interval_family.unit(), TimeUnit::DayTime);
        assert_eq!(DataType::from(interval_family), interval);

        let text = DataType::large_utf8().string_parameters().unwrap();
        assert_eq!(text, StringType::LargeUtf8String);
        assert_eq!(DataType::string(text).unwrap(), DataType::large_utf8());

        let encoded = StringType::Utf8StringView
            .with_charset(Charset::Cp1252)
            .unwrap();
        assert_eq!(encoded, StringType::Cp1252StringView);
        assert_eq!(
            DataType::string(encoded).unwrap().string_parameters(),
            Some(encoded)
        );

        // The string family has no family enum of its own: `DataTypeId` names
        // the exact leaf and `string_parameters` answers it, charset and number
        // included, so a third listing would only be one more thing to disagree.
        let ascii = DataType::fixed_ascii(7).unwrap();
        assert_eq!(ascii.id(), yggdryl::DataTypeId::FixedAsciiString);
        assert_eq!(ascii.fixed_byte_width(), Some(7));
        assert!(ascii.is_string());
        let parameters = ascii.string_parameters().unwrap();
        assert_eq!(parameters, StringType::FixedAsciiString(7));
        assert_eq!(parameters.charset(), Charset::Ascii);
        assert_eq!(parameters.fixed(), Some(7));
        assert_eq!(DataType::string(parameters).unwrap(), ascii);

        // The byte family reads back the same way: the layout and the bound.
        let bytes = DataType::fixed_binary(16).unwrap();
        assert_eq!(bytes.id(), yggdryl::DataTypeId::FixedBinary);
        assert_eq!(bytes.fixed_byte_width(), Some(16));
        let parameters = bytes.bytes_parameters().unwrap();
        assert_eq!(parameters, BytesType::FixedBinary(16));
        assert_eq!(parameters.fixed(), Some(16));
        assert_eq!(DataType::bytes(parameters).unwrap(), bytes);
        assert_eq!(DataType::utf8().bytes_parameters(), None);

        // A wrapper datatype answers for itself now that no intermediate enum
        // stands between the root and the families.
        let nested = DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap();
        assert!(nested.id().is_wrapper());
        assert_eq!(nested.id(), yggdryl::DataTypeId::Dictionary);

        // A geometry and a geography each hold their parameters, and the
        // payload of one is never the other's.
        let geospatial = DataType::geography(None, None).unwrap();
        let geography = GeographyType::from_dtype(&geospatial).unwrap();
        assert_eq!(geography.into_dtype(), geospatial);
        assert!(GeometryType::from_dtype(&geospatial).is_none());
        assert!(DataTypeKind::Geospatial.contains(geospatial.id()));

        assert!(!DataType::utf8().is_integer());
    }

    #[test]
    fn nested_helper_values_have_total_order_and_hash() {
        fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
        assert_traits::<MapType>();
        assert_traits::<RunEndEncodedType>();

        let first = DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap();
        let later = DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap();
        assert!(first < later);
        let (DataType::Map(unsorted), DataType::SortedMap(sorted)) = (&first, &later) else {
            unreachable!()
        };
        assert_eq!(unsorted, sorted);

        let first = DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Int32, true),
        )
        .unwrap();
        let later = DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Int64, true),
        )
        .unwrap();
        let (DataType::RunEndEncoded(first), DataType::RunEndEncoded(later)) = (first, later)
        else {
            unreachable!()
        };
        assert!(first < later);
    }

    #[test]
    #[allow(clippy::mutable_key_type)] // Field caches are excluded from all value traits.
    fn native_order_hash_and_child_access_are_value_based() {
        let left = DataType::from_str("struct<a:int32,b:string>").unwrap();
        let right = DataType::from_str("struct<a:int64,b:string>").unwrap();
        let ordered = BTreeSet::from([right.clone(), left.clone()]);
        let hashed = HashSet::from([right.clone(), left.clone()]);

        assert_eq!(ordered.len(), 2);
        assert_eq!(hashed.len(), 2);
        assert_ne!(left.stable_hash(), right.stable_hash());
        assert_eq!(left.field_len(), 2);
        assert_eq!(left.get_field(0).map(Field::name), Some("a"));
        assert_eq!(left.get_field_by_path("b").map(Field::name), Some("b"));
        assert_eq!(left.as_fields().map(<[Field]>::len), Some(2));
    }

    #[test]
    fn every_arrow_variant_has_a_lossless_owned_equivalent() {
        let item = || Field::new("item", DataType::utf8(), true);
        let values = vec![
            DataType::Null,
            DataType::Boolean,
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::Float16,
            DataType::Float32,
            DataType::Float64,
            DataType::datetime64(
                TimeUnit::Nanosecond,
                yggdryl::Timezone::from_str("Europe/Paris").unwrap(),
            )
            .unwrap(),
            DataType::date32(),
            DataType::date64(),
            DataType::time32(TimeUnit::Millisecond).unwrap(),
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            DataType::duration64(TimeUnit::Nanosecond).unwrap(),
            DataType::interval(TimeUnit::YearMonth).unwrap(),
            DataType::interval(TimeUnit::DayTime).unwrap(),
            DataType::interval(TimeUnit::MonthDayNano).unwrap(),
            DataType::binary(),
            DataType::fixed_binary(16).unwrap(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::serie(item()),
            DataType::serie_view(item()),
            DataType::fixed_size_serie(item(), 4).unwrap(),
            DataType::large_serie(item()),
            DataType::large_serie_view(item()),
            DataType::from(
                StructType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
            ),
            DataType::union(
                [
                    (0, Field::new("number", DataType::Int64, false)),
                    (1, Field::new("text", DataType::utf8(), true)),
                ],
                UnionMode::Sparse,
            )
            .unwrap(),
            DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
            DataType::decimal32(9, 2).unwrap(),
            DataType::decimal64(18, -2).unwrap(),
            DataType::decimal128(38, 18).unwrap(),
            DataType::decimal256(76, 20).unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
        ];

        for source in values {
            let arrow: ArrowDataType = source.clone().into_arrow_datatype().unwrap();
            let arrow_debug = format!("{arrow:?}");
            let parsed_debug = DataType::from_str(&arrow_debug)
                .unwrap_or_else(|error| panic!("failed to parse {arrow_debug}: {error}"));
            assert_eq!(
                parsed_debug, source,
                "failed Arrow Debug parse: {arrow_debug}"
            );
            let restored = DataType::try_from(arrow).unwrap();
            assert_eq!(restored, source, "failed round-trip for {source}");
        }
    }

    #[test]
    fn arrow_import_enforces_one_shared_recursion_budget() {
        fn nested_list(levels: usize) -> ArrowDataType {
            let mut value = ArrowDataType::Int64;
            for _ in 0..levels {
                value = ArrowDataType::List(Arc::new(ArrowField::new("item", value, true)));
            }
            value
        }

        let maximum = nested_list(DataType::PARSE_RECURSION_LIMIT - 1);
        assert!(DataType::from_arrow_datatype(&maximum).is_ok());
        assert!(DataType::try_from(maximum).is_ok());

        let over_limit = nested_list(DataType::PARSE_RECURSION_LIMIT);
        assert!(DataType::from_arrow_datatype(&over_limit).is_err());
        assert!(DataType::try_from(over_limit).is_err());
    }
}

/// A datatype hashes as the shape it had before the text and byte leaves
/// each took a variant of their own, so no stored digest over one moves.
///
/// Every value here was taken from the build before the split, through the
/// stable sink every digest writes into; `Display` names each datatype so the
/// table outlives any constructor. Every variant is in it, every string and
/// byte leaf at two numbers, and a dictionary, a map and a serie over them:
/// a dictionary's value type is how the discriminant reaches a stored digest.
#[cfg(feature = "internals")]
mod structural_hash {
    use yggdryl::internals::hashing_stable::stable_hash_of;
    use yggdryl::{DataType, Scalar};

    const PINNED: [(&str, u64); 101] = [
        ("utf8", 0x5ab6cab83f73e718),
        ("large_utf8", 0xc049e53a48cbfe3b),
        ("utf8_view", 0xf4813115041b88d1),
        ("large_utf8_view", 0x2913339cc7b51d1a),
        ("fixed_utf8(7)", 0x73565b305d96e374),
        ("sized_utf8(7)", 0x640514f72e636877),
        ("ascii", 0x502ad1b174fac85a),
        ("large_ascii", 0x57001e60b1030761),
        ("ascii_view", 0x3522740576c708f7),
        ("large_ascii_view", 0xc1a4bcead29bf9dd),
        ("fixed_ascii(7)", 0x0bd7750bd3fdcac2),
        ("sized_ascii(7)", 0x490685a23812fb74),
        ("cp1252", 0x912c41c4998c089f),
        ("large_cp1252", 0x751c0f6293b51839),
        ("cp1252_view", 0xb31b7c0a8df9aaf5),
        ("large_cp1252_view", 0x47ebd43677182d8b),
        ("fixed_cp1252(7)", 0x5bd5b6fde721f3cc),
        ("sized_cp1252(7)", 0xa834da62b05f4266),
        ("binary", 0xb4f46a1bc25589ed),
        ("large_binary", 0x144130a4eb9b36cc),
        ("binary_view", 0x512742e6e2ee1bbb),
        ("large_binary_view", 0x58ba28841599c919),
        ("fixed_binary(9)", 0xa904efcec65abcae),
        ("sized_binary(9)", 0x6f3d1efdae61cba1),
        ("fixed_utf8(300)", 0x3a3d75282cd92a74),
        ("sized_utf8(300)", 0x5ec93e6edd75faa8),
        ("fixed_ascii(300)", 0xf5ff507c073b272a),
        ("sized_ascii(300)", 0xe5883e398a94bf36),
        ("fixed_cp1252(300)", 0x8aafb890cb7384e5),
        ("sized_cp1252(300)", 0xca2af0cceba9471c),
        ("fixed_binary(302)", 0x6491a68f401fd149),
        ("sized_binary(302)", 0x47581d086e672b99),
        ("null", 0xc77b3abb6f87acd9),
        ("boolean", 0x2fbc593564db792e),
        ("int8", 0x2086c65c91eee243),
        ("int16", 0x4d922029c1f42e7d),
        ("int32", 0xca22290ad95e7178),
        ("int64", 0x8e03e9aa39aaa78c),
        ("uint8", 0x6b994bda4763673b),
        ("uint16", 0x81671e58d6b596af),
        ("uint32", 0xa1fb3d150676dcfe),
        ("uint64", 0x0760af8819750497),
        ("float16", 0xe6ad1be9a8972875),
        ("float32", 0x2abb136b5b23df0c),
        ("float64", 0x9e9e459506814997),
        ("datetime64(ns,\"UTC\")", 0x201d04a8bdf2caea),
        ("datetime64(ms)", 0x3a14388b75fca7bb),
        ("date32", 0x311479a7c6e57836),
        ("date64", 0xe5ce56c10171f463),
        ("time32(ms)", 0x72e64251ed9fd5ad),
        ("time32(s)", 0x1e6a9280c8839372),
        ("time64(ns)", 0x4a584b2f9fae9b66),
        ("duration32(s)", 0x7a17a809856e8031),
        ("duration64(us)", 0xa34cad317dd01868),
        ("interval(month_day_nano)", 0x65a9b60070c1a0c8),
        ("interval(year_month)", 0xc73891afed11ae2a),
        ("interval(day_time)", 0x7ad288a441748064),
        ("country", 0xe41438cbe011bd56),
        ("ccy", 0xbe7f9506a52d0949),
        ("mic", 0xd7ead9fce536323e),
        ("cfi", 0xd97e41930d68e393),
        ("isin", 0x0f222354ded30363),
        ("side", 0x52a98a94618c687b),
        ("state", 0x384fa1ab24cdaafb),
        ("timeinforce", 0x5e9bed84925ef4fe),
        ("uuid", 0x126c911693422108),
        ("version", 0xe2fca2fc6cd1c60d),
        ("url", 0x7e1ee45fc1090ac6),
        ("urn", 0xeec41aec8a47156d),
        (
            "serie(field(\"item\",int64,nullable=true,metadata={}))",
            0xcb153063d2c38cb6,
        ),
        (
            "serie(field(\"item\",utf8,nullable=true,metadata={}))",
            0x42f06ab5d1b927e2,
        ),
        (
            "serie_view(field(\"item\",ascii,nullable=true,metadata={}))",
            0x5bd08201d94be48d,
        ),
        (
            "fixed_size_serie(field(\"item\",binary,nullable=true,metadata={}),3)",
            0x56bef4dff6aab557,
        ),
        (
            "large_serie(field(\"item\",large_utf8,nullable=true,metadata={}))",
            0xbfe28c04dbea577b,
        ),
        (
            "large_serie_view(field(\"item\",sized_cp1252(5),nullable=true,metadata={}))",
            0xe5807a3fc2cee680,
        ),
        (
            "struct(field(\"a\",utf8,nullable=true,metadata={}),field(\"b\",sized_binary(4),nullable=true,metadata={}),field(\"c\",fixed_ascii(3),nullable=true,metadata={}))",
            0xa82a10c450bbd93d,
        ),
        (
            "union(sparse,0=field(\"a\",int8,nullable=true,metadata={}),1=field(\"b\",utf8,nullable=true,metadata={}))",
            0xe79e2a7b21eef18e,
        ),
        ("dictionary(int32,large_utf8)", 0xc7eddf242e896a89),
        ("dictionary(int8,fixed_ascii(3))", 0x5ab835d87840b116),
        ("dictionary(int16,binary_view)", 0x30bead153d23bc1d),
        ("decimal32(9,2)", 0x8ed9284eef26f840),
        ("decimal64(18,4)", 0x04c18e7bb778cc69),
        ("decimal128(38,10)", 0xe69e5a10d7312cdb),
        ("decimal256(50,3)", 0xf4c1e25784ae9060),
        (
            "map(field(\"entries\",struct(field(\"key\",utf8,nullable=false,metadata={}),field(\"value\",binary,nullable=true,metadata={})),nullable=false,metadata={}),keys_sorted=false)",
            0x033cdec8125a26be,
        ),
        (
            "map(field(\"entries\",struct(field(\"key\",cp1252,nullable=false,metadata={}),field(\"value\",fixed_binary(4),nullable=true,metadata={})),nullable=false,metadata={}),keys_sorted=false)",
            0xdbc028340604ff2e,
        ),
        (
            "run_end_encoded(field(\"run_ends\",int32,nullable=false,metadata={}),field(\"values\",utf8,nullable=true,metadata={}))",
            0x40689b236c921e61,
        ),
        ("variant", 0x8c9234f7f2a46cca),
        ("geometry", 0x640f19434179cf8c),
        ("geography", 0x7767dd6ad1fdfef9),
        ("timezone", 0x8f236ea5f57db974),
        ("mimetype", 0x0e06ee9d32968676),
        ("mediatype", 0x5a5bfcc13afb0644),
        ("cusip", 0xc15ec86c30fae930),
        ("sedol", 0x7c79bb6940cec97c),
        ("bbg", 0x0c4abe1a4bc0cfb3),
        ("figi", 0x42a68c6154ca5261),
        (
            "map(field(\"entries\",struct(field(\"key\",ascii,nullable=false,metadata={}),field(\"value\",sized_binary(4),nullable=true,metadata={})),nullable=false,metadata={}),keys_sorted=true)",
            0x820f074fa9fd900f,
        ),
        (
            "map(field(\"entries\",struct(field(\"key\",ascii,nullable=false,metadata={}),field(\"value\",sized_binary(4),nullable=true,metadata={})),nullable=false,metadata={}),keys_sorted=false)",
            0x1d162143e847fd91,
        ),
        // Appended when the unit code landed: the `Shape` arm sits at the end
        // of that enum, so every row above keeps its value.
        ("unit", 0x9c1c8fe3b6bd3cdb),
        // Appended when the RIC code landed, its `Shape` arm last in turn.
        ("ric", 0x73db933494b531da),
    ];

    #[test]
    fn every_datatype_hashes_as_it_did_before_the_leaves_split() {
        for (name, pinned) in PINNED {
            let dtype = DataType::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(dtype.to_string(), name, "{name} spells itself");
            assert_eq!(stable_hash_of(&dtype), pinned, "{name}");
        }
    }

    #[test]
    fn a_value_hashes_its_text_or_payload_whichever_leaf_holds_it() {
        for (text, leaf, pinned) in [
            ("abc", "utf8", 0x20338bf0cfb053f7),
            ("abc", "large_ascii", 0x20338bf0cfb053f7),
            ("ab", "fixed_cp1252(4)", 0x2b6321a650d6efb8),
            ("ab", "sized_utf8(9)", 0x2b6321a650d6efb8),
        ] {
            let value = DataType::from_str(leaf)
                .unwrap()
                .scalar(Scalar::from(text))
                .unwrap();
            assert_eq!(
                value.string_parameters().map(DataType::from),
                Some(DataType::from_str(leaf).unwrap())
            );
            assert_eq!(stable_hash_of(&value), pinned, "{text} as {leaf}");
        }
        for (payload, leaf, pinned) in [
            (&b"abc"[..], "binary", 0x26a87cc23ddaf6a5),
            (&b"abc"[..], "fixed_binary(3)", 0x26a87cc23ddaf6a5),
            (&b"ab"[..], "sized_binary(9)", 0xa7122a6c11fcece3),
            (&b"ab"[..], "large_binary_view", 0xa7122a6c11fcece3),
        ] {
            let value = DataType::from_str(leaf)
                .unwrap()
                .scalar(Scalar::from(payload.to_vec()))
                .unwrap();
            assert_eq!(
                value.bytes_parameters().map(DataType::from),
                Some(DataType::from_str(leaf).unwrap())
            );
            assert_eq!(stable_hash_of(&value), pinned, "{payload:?} as {leaf}");
        }
    }
}
