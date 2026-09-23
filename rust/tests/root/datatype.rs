//! `rust/src/datatype.rs`.

mod arrow {
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use yggdryl::BytesType;

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
            DataType::list(item()),
            DataType::list_view(item()),
            DataType::fixed_size_list(item(), 4).unwrap(),
            DataType::large_list(item()),
            DataType::large_list_view(item()),
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
            DataType::Currency,
            DataType::MicCode,
            DataType::CfiCode,
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
                DataType::list(Field::new("item", dtype.clone(), true)),
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
        assert!(DataType::fixed_size_list(Field::new("item", DataType::utf8(), true), -1).is_err());
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
        let invalid_binary = DataType::Bytes(BytesType::FixedBinary(0));
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
        let invalid_list = DataType::FixedSizeList(Arc::new(item.clone()), -1);
        for error in [
            DataType::fixed_size_list(item, -1).unwrap_err(),
            invalid_list.validate().unwrap_err(),
            invalid_list.clone().into_arrow_datatype().unwrap_err(),
            invalid_list.into_arrow_datatype_ffi().unwrap_err(),
        ] {
            assert_invalid(error, "FixedSizeList", "length must be non-negative: -1");
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
            DataType::Currency,
            DataType::MicCode,
            DataType::CfiCode,
            DataType::IsinCode,
            DataType::CusipCode,
            DataType::SedolCode,
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
            DataType::list(item()),
            DataType::list_view(item()),
            DataType::fixed_size_list(item(), 4).unwrap(),
            DataType::large_list(item()),
            DataType::large_list_view(item()),
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
