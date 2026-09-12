use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::types::{BytesLayout, BytesParameters};
use yggdryl::{DataType, Field, TimeUnit, Timezone, UnionMode};

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
        DataType::from_str("struct<id:bigint,values:map<string,array<decimal(38,18)>>>").unwrap();
    let borrowed = dtype.clone().into_arrow().unwrap();
    assert_eq!(DataType::from_arrow(&borrowed).unwrap(), dtype);
    let owned = dtype.clone().into_arrow().unwrap();
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
    let core = DataType::from_arrow(&arrow).unwrap();
    assert_eq!(core.clone().into_arrow().unwrap(), arrow);
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
        let arrow = value.clone().into_arrow().unwrap();
        assert_eq!(DataType::from_arrow(&arrow).unwrap(), value);
        assert_eq!(DataType::try_from(arrow.clone()).unwrap(), value);
        assert_eq!(value.clone().into_arrow().unwrap(), arrow);

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
        let arrow = narrow.into_arrow().unwrap();
        assert_eq!(
            arrow,
            ArrowDataType::Duration(unit.into_arrow_time().unwrap())
        );
        assert_eq!(
            DataType::from_arrow(&arrow).unwrap(),
            DataType::Duration64(unit)
        );
    }
}

#[test]
fn every_arrow_datatype_variant_round_trips_borrowed_owned_display_json_and_debug() {
    let item = || Field::new("item", DataType::utf8(), true);
    let entries = || {
        Field::new(
            "entries",
            DataType::from_fields([
                Field::new("key", DataType::utf8(), false),
                Field::new("value", DataType::Int64, true),
            ])
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
        DataType::Date32,
        DataType::Date64,
        DataType::Time32(TimeUnit::Millisecond),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration64(TimeUnit::Nanosecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::binary(),
        DataType::fixed_size_binary(16).unwrap(),
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
        DataType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
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
        let borrowed = value.clone().into_arrow().unwrap();
        let ffi = value.clone().into_arrow_ffi().unwrap();
        assert_eq!(ArrowDataType::try_from(&ffi).unwrap(), borrowed);
        assert_eq!(DataType::from_arrow(&borrowed).unwrap(), value);
        assert_eq!(DataType::try_from(borrowed.clone()).unwrap(), value);
        assert_eq!(value.clone().into_arrow().unwrap(), borrowed);
        assert_eq!(DataType::from_str(&value.to_string()).unwrap(), value);
        assert_eq!(
            DataType::from_json(&value.clone().into_json().unwrap()).unwrap(),
            value
        );
        let debug = format!("{borrowed:?}");
        let parsed = DataType::from_str(&debug)
            .unwrap_or_else(|error| panic!("failed to parse Arrow debug form {debug}: {error}"));
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
            DataType::list(Field::new("item", dtype.clone(), true)),
            DataType::from_fields([Field::new("child", dtype.clone(), true)]).unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", dtype.clone(), true),
            )
            .unwrap(),
        ] {
            let field = Field::new("f", held.clone(), true);
            let arrow = field.clone().into_arrow().unwrap();
            assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{held}");

            // The C schema is a field node too, so it carries the same
            // identity, dictionary encoding included.
            let ffi = field.clone().into_arrow_ffi().unwrap();
            let imported = ArrowField::try_from(&ffi).unwrap();
            assert_eq!(
                Field::from_arrow(&imported).unwrap().dtype(),
                &held,
                "{held}"
            );

            // A bare datatype has nowhere to carry it in Arrow, but its own C
            // schema does.
            let ffi = held.clone().into_arrow_ffi().unwrap();
            let imported = ArrowField::try_from(&ffi).unwrap();
            assert_eq!(
                Field::from_arrow(&imported).unwrap().dtype(),
                &held,
                "{held}"
            );
        }

        // The documented exception: an Arrow datatype is storage, because it
        // has no metadata to name an extension with.
        let storage = dtype.clone().into_arrow().unwrap();
        assert_ne!(DataType::from_arrow(&storage).unwrap(), dtype, "{dtype}");
    }
}

#[test]
fn an_extension_schema_survives_an_ipc_round_trip() {
    let root = DataType::from_fields(
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
        assert!(invalid.clone().into_arrow().is_err());
        assert!(invalid.into_json().is_err());
    }
    assert!(DataType::fixed_size_binary(0).is_err());
    assert!(DataType::fixed_size_list(Field::new("item", DataType::utf8(), true), -1).is_err());
    assert!(DataType::decimal128(0, 0).is_err());
    assert!(DataType::decimal128(5, 6).is_err());
    assert!(DataType::dictionary(DataType::Float64, DataType::utf8()).is_err());
    assert!(
        DataType::map(
            Field::new(
                "entries",
                DataType::from_fields([
                    Field::new("key", DataType::utf8(), true),
                    Field::new("value", DataType::Int64, true),
                ])
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
    assert!(DataType::from_arrow(&duplicate_arrow).is_err());
    assert!(DataType::try_from(duplicate_arrow).is_err());
}

#[test]
fn invariant_errors_match_across_construction_validation_and_arrow_projection() {
    let invalid_time = DataType::Time32(TimeUnit::Nanosecond);
    for error in [
        DataType::time32(TimeUnit::Nanosecond).unwrap_err(),
        invalid_time.validate().unwrap_err(),
        invalid_time.clone().into_arrow().unwrap_err(),
        invalid_time.into_arrow_ffi().unwrap_err(),
    ] {
        assert_invalid(error, "Time32", "unit must be second or millisecond");
    }

    // The constructor refuses a width of zero; the variant is public, so a
    // fixed layout can also be built with no width at all, and every door
    // past construction refuses that one alike.
    assert_invalid(
        DataType::fixed_size_binary(0).unwrap_err(),
        "bytes",
        "expected a width of at least one byte, got 0",
    );
    let invalid_binary = DataType::Bytes(BytesParameters::new(BytesLayout::FixedSizeBinary));
    for error in [
        invalid_binary.validate().unwrap_err(),
        invalid_binary.clone().into_arrow().unwrap_err(),
        invalid_binary.into_arrow_ffi().unwrap_err(),
    ] {
        assert_invalid(
            error,
            "bytes",
            "expected fixed_size_binary(width), got no width",
        );
    }

    let item = Field::new("item", DataType::utf8(), true);
    let invalid_list = DataType::FixedSizeList(Arc::new(item.clone()), -1);
    for error in [
        DataType::fixed_size_list(item, -1).unwrap_err(),
        invalid_list.validate().unwrap_err(),
        invalid_list.clone().into_arrow().unwrap_err(),
        invalid_list.into_arrow_ffi().unwrap_err(),
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
        DataType::Mic,
        DataType::Cfi,
        DataType::Isin,
        DataType::Side,
        DataType::State,
        DataType::TimeInForce,
        DataType::Uuid,
        DataType::Version,
        DataType::Url,
        DataType::Variant,
        DataType::from_str("string(windows-1252)").unwrap(),
        DataType::from_str("fixed_string(windows-1252,8)").unwrap(),
        DataType::from_str("large_utf8_view").unwrap(),
    ];

    for dtype in extension_typed {
        let ffi = dtype.clone().into_arrow_ffi().unwrap();
        let arrow = ArrowField::try_from(&ffi)
            .unwrap_or_else(|error| panic!("{dtype} did not project a C schema: {error}"));
        let name = arrow
            .metadata()
            .get("ARROW:extension:name")
            .unwrap_or_else(|| panic!("{dtype} crossed the C Data Interface without its identity"));
        assert!(
            name.starts_with("yggdryl.") || name.starts_with("arrow.") || name.contains('.'),
            "{dtype} projected an unqualified extension name {name:?}"
        );
        // And what came back reads as the datatype that was sent.
        assert_eq!(
            Field::from_arrow(&arrow).unwrap().dtype(),
            &dtype,
            "{dtype} did not read back as itself"
        );
    }
}
