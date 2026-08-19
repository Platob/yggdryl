use std::io::Cursor;
use std::sync::Arc;

use yggdryl::{DataType, Error, Field, Scheme, TimeUnit, Timezone, UnionMode};

#[test]
fn arrow_is_a_cache_preserving_validated_noop() {
    let field = Field::from_parts(
        "value",
        DataType::from_fields([Field::new("child", DataType::Utf8, true)]).unwrap(),
        false,
        [("owner", "yggdryl")],
    )
    .unwrap();
    let cached = field.to_arrow_ref().unwrap();
    let compatible = field.to_scheme_compat(&Scheme::ARROW).unwrap();

    assert_eq!(compatible, field);
    assert!(Arc::ptr_eq(&cached, &compatible.to_arrow_ref().unwrap()));
}

#[test]
fn spark_applies_only_the_conservative_recursive_matrix() {
    let source = DataType::from_fields([
        Field::new("small", DataType::UInt8, false),
        Field::new("wide", DataType::UInt64, true),
        Field::new(
            "items",
            DataType::large_list(Field::new("item", DataType::Utf8View, true)),
            false,
        ),
        Field::new(
            "encoded",
            DataType::dictionary(DataType::Int8, DataType::LargeBinary).unwrap(),
            false,
        ),
    ])
    .unwrap();
    let transformed = source.to_scheme_compat(&Scheme::SPARK).unwrap();
    let fields = transformed.as_fields().unwrap();
    assert_eq!(fields[0].data_type(), &DataType::Int16);
    assert_eq!(fields[1].data_type(), &DataType::decimal128(20, 0).unwrap());
    let DataType::List(item) = fields[2].data_type() else {
        panic!("expected normalized list");
    };
    assert_eq!(item.data_type(), &DataType::Utf8);
    assert!(item.is_nullable());
    assert_eq!(fields[3].data_type(), &DataType::Binary);
    assert!(fields[1].is_nullable());
}

#[test]
fn spark_physical_rewrite_table_covers_offset_numeric_and_decimal_families() {
    let cases = vec![
        (DataType::UInt16, DataType::Int32),
        (DataType::UInt32, DataType::Int64),
        (DataType::Float16, DataType::Float32),
        (DataType::fixed_size_binary(8).unwrap(), DataType::Binary),
        (DataType::LargeBinary, DataType::Binary),
        (DataType::BinaryView, DataType::Binary),
        (DataType::LargeUtf8, DataType::Utf8),
        (DataType::Utf8View, DataType::Utf8),
        (
            DataType::list_view(Field::new("item", DataType::UInt16, true)),
            DataType::list(Field::new("item", DataType::Int32, true)),
        ),
        (
            DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 4).unwrap(),
            DataType::list(Field::new("item", DataType::Int32, false)),
        ),
        (
            DataType::large_list_view(Field::new("item", DataType::Utf8View, true)),
            DataType::list(Field::new("item", DataType::Utf8, true)),
        ),
        (
            DataType::decimal32(7, 2).unwrap(),
            DataType::decimal128(7, 2).unwrap(),
        ),
        (
            DataType::decimal64(12, 2).unwrap(),
            DataType::decimal128(12, 2).unwrap(),
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            source.to_scheme_compat(&Scheme::SPARK).unwrap(),
            expected,
            "unexpected Spark projection for {source:?}"
        );
    }
}

#[test]
fn spark_errors_are_path_aware_and_extension_rewrites_are_atomic() {
    let source = DataType::from_fields([Field::new(
        "a.b",
        DataType::Timestamp(TimeUnit::Nanosecond, None),
        false,
    )])
    .unwrap();
    let error = source
        .to_scheme_compat(&Scheme::SPARK)
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[\"a.b\"]"), "{error}");

    let extension = Field::from_parts(
        "value",
        DataType::UInt8,
        true,
        [("ARROW:extension:name", "example.u8")],
    )
    .unwrap();
    let before = extension.clone();
    let error = extension.to_scheme_compat(&Scheme::SPARK).unwrap_err();
    assert!(matches!(error, Error::InvalidDataType { .. }));
    assert_eq!(extension, before);

    let no_op_extension = Field::from_parts(
        "value",
        DataType::Utf8,
        true,
        [("ARROW:extension:name", "example.text")],
    )
    .unwrap();
    assert_eq!(
        no_op_extension.to_scheme_compat(&Scheme::SPARK).unwrap(),
        no_op_extension
    );
}

#[test]
fn compatibility_targets_share_the_canonical_scheme_parser() {
    assert_eq!(Scheme::from_str("SPARK").unwrap(), Scheme::SPARK);
    assert_eq!(Scheme::from_str("Polars").unwrap(), Scheme::POLARS);
    assert_eq!(Scheme::from_str("pandas").unwrap(), Scheme::PANDAS);
    assert_eq!(serde_json::to_string(&Scheme::ARROW).unwrap(), r#""arrow""#);
    assert_eq!(
        serde_json::from_value::<Scheme>(serde_json::json!("spark")).unwrap(),
        Scheme::SPARK
    );
    assert_eq!(
        serde_json::from_reader::<_, Scheme>(Cursor::new(br#""arrow""#)).unwrap(),
        Scheme::ARROW
    );

    for target in Scheme::COMPATIBILITY_TARGETS {
        assert!(target.is_compatibility_target(), "{target}");
    }

    // Iceberg is a metadata namespace *and* a normalization target, so it has
    // to appear in the one central list rather than only inside the module.
    assert!(Scheme::COMPATIBILITY_TARGETS.contains(&Scheme::ICEBERG));
    assert!(Scheme::ICEBERG.is_compatibility_target());
    assert_eq!(Scheme::from_str("Iceberg").unwrap(), Scheme::ICEBERG);
}

#[test]
fn a_non_compatibility_scheme_is_rejected_by_normalization_not_by_parsing() {
    // `Scheme` is an open URI vocabulary, so an unrelated scheme still parses.
    let duckdb = Scheme::from_str("duckdb").unwrap();
    assert_eq!(duckdb.as_str(), "duckdb");
    assert!(!duckdb.is_compatibility_target());

    // Rejection happens where the target is actually used, and the message
    // names both the accepted vocabulary and the offending value.
    let error = DataType::Int32.to_scheme_compat(&duckdb).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("\"duckdb\""), "{message}");
    assert!(message.contains("arrow"), "{message}");
    assert!(message.contains("spark"), "{message}");
    assert!(message.contains("polars"), "{message}");
    assert!(message.contains("pandas"), "{message}");
    assert!(message.contains("iceberg"), "{message}");
    assert!(matches!(
        error,
        Error::InvalidDataType {
            kind: "Compatibility",
            ..
        }
    ));

    let field_error = Field::new("value", DataType::Int32, true)
        .to_scheme_compat(&duckdb)
        .unwrap_err();
    assert!(matches!(
        field_error,
        Error::InvalidDataType {
            kind: "Compatibility",
            ..
        }
    ));
}

#[test]
fn polars_keeps_unsigned_integers_and_fixed_size_lists() {
    // Spark widens unsigned integers; Polars has them natively.
    assert_eq!(
        DataType::UInt32.to_scheme_compat(&Scheme::POLARS).unwrap(),
        DataType::UInt32
    );
    assert_eq!(
        DataType::UInt32.to_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::Int64
    );

    // Polars `Array` keeps the fixed-length layout; Spark degrades to a list.
    let fixed = DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 3).unwrap();
    assert_eq!(fixed.to_scheme_compat(&Scheme::POLARS).unwrap(), fixed);
    assert_eq!(
        fixed.to_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::list(Field::new("item", DataType::Int32, false))
    );
}

#[test]
fn polars_and_pandas_reject_maps_with_a_named_alternative() {
    let map = DataType::map(
        Field::new(
            "entries",
            DataType::from_fields(vec![
                Field::new("key", DataType::Utf8, false),
                Field::new("value", DataType::Int64, true),
            ])
            .unwrap(),
            false,
        ),
        false,
    )
    .unwrap();

    // Spark has a first-class map and keeps it.
    assert!(map.to_scheme_compat(&Scheme::SPARK).is_ok());

    for target in [Scheme::POLARS, Scheme::PANDAS] {
        let message = map.to_scheme_compat(&target).unwrap_err().to_string();
        assert!(message.contains("no first-class map type"), "{message}");
        assert!(message.contains("key/value structs"), "{message}");
    }
}

#[test]
fn temporal_resolution_errors_name_the_expected_and_actual_unit() {
    let nanosecond = DataType::Timestamp(TimeUnit::Nanosecond, None);

    // pandas is nanosecond-native, Spark is microsecond-native.
    assert_eq!(
        nanosecond.to_scheme_compat(&Scheme::PANDAS).unwrap(),
        nanosecond
    );
    let message = nanosecond
        .to_scheme_compat(&Scheme::SPARK)
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected timestamp of us"), "{message}");
    assert!(message.contains("got ns"), "{message}");
    assert!(message.contains("value cast"), "{message}");

    let second = DataType::Timestamp(TimeUnit::Second, None);
    let polars_message = second
        .to_scheme_compat(&Scheme::POLARS)
        .unwrap_err()
        .to_string();
    assert!(polars_message.contains("got s"), "{polars_message}");
    assert!(polars_message.contains("ms, us, or ns"), "{polars_message}");
}

#[test]
fn every_target_reports_a_path_for_a_nested_failure() {
    let nested = DataType::from_fields(vec![Field::new(
        "outer",
        DataType::list(Field::new(
            "item",
            DataType::Timestamp(TimeUnit::Second, None),
            true,
        )),
        true,
    )])
    .unwrap();

    for target in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
        let message = nested.to_scheme_compat(&target).unwrap_err().to_string();
        assert!(message.contains("outer"), "{target}: {message}");
        assert!(message.contains("[]"), "{target}: {message}");
        assert!(message.contains("got s"), "{target}: {message}");
    }
}

#[test]
fn negative_decimal_scale_names_the_offending_scale() {
    let negative = DataType::Decimal128 {
        precision: 10,
        scale: -2,
    };
    for target in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
        let message = negative.to_scheme_compat(&target).unwrap_err().to_string();
        assert!(
            message.contains("expected a non-negative decimal scale, got -2"),
            "{target}: {message}"
        );
    }
}

#[test]
fn compatibility_preflight_reports_its_own_operation_kind() {
    let mut nested = DataType::Int32;
    for _ in 0..DataType::PARSE_RECURSION_LIMIT {
        nested = DataType::list(Field::new("item", nested, false));
    }
    for (scheme, expected_kind) in [
        (Scheme::ARROW, "ArrowCompatibility"),
        (Scheme::SPARK, "SparkCompatibility"),
        (Scheme::POLARS, "PolarsCompatibility"),
        (Scheme::PANDAS, "PandasCompatibility"),
        (Scheme::ICEBERG, "IcebergCompatibility"),
    ] {
        let error = nested.to_scheme_compat(&scheme).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidDataType { kind, .. } if kind == expected_kind
        ));
    }
}

#[test]
fn spark_temporal_decimal_and_union_boundaries_are_explicit() {
    for accepted in [
        DataType::Timestamp(TimeUnit::Microsecond, Some(Timezone::UTC)),
        DataType::Duration(TimeUnit::Microsecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::decimal128(38, 0).unwrap(),
    ] {
        assert_eq!(accepted.to_scheme_compat(&Scheme::SPARK).unwrap(), accepted);
    }
    for rejected in [
        DataType::Timestamp(TimeUnit::Nanosecond, None),
        DataType::Date64,
        DataType::Time32(TimeUnit::Second),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration(TimeUnit::Nanosecond),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::decimal128(9, -1).unwrap(),
        DataType::decimal256(39, 0).unwrap(),
        DataType::union(
            [(1, Field::new("value", DataType::Int32, false))],
            UnionMode::Dense,
        )
        .unwrap(),
    ] {
        assert!(
            rejected.to_scheme_compat(&Scheme::SPARK).is_err(),
            "{rejected:?}"
        );
    }
}

#[test]
fn spark_recurses_through_map_dictionary_and_run_end_layouts() {
    let map = DataType::map_of(DataType::Utf8View, DataType::UInt8, true).unwrap();
    let transformed = map.to_scheme_compat(&Scheme::SPARK).unwrap();
    let DataType::Map(map) = transformed else {
        panic!("expected map");
    };
    assert!(map.keys_sorted());
    let fields = map.entries().data_type().as_fields().unwrap();
    assert_eq!(fields[0].data_type(), &DataType::Utf8);
    assert_eq!(fields[1].data_type(), &DataType::Int16);

    let dictionary = DataType::dictionary(
        DataType::Int16,
        DataType::list(Field::new("item", DataType::UInt16, true)),
    )
    .unwrap();
    let transformed = dictionary.to_scheme_compat(&Scheme::SPARK).unwrap();
    let DataType::List(item) = transformed else {
        panic!("expected logical dictionary list");
    };
    assert_eq!(item.data_type(), &DataType::Int32);

    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Utf8View, true),
    )
    .unwrap();
    assert_eq!(
        encoded.to_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::Utf8
    );
}

#[test]
fn spark_changed_fields_preserve_value_state_and_invalidate_cache_once() {
    let mut field = Field::from_parts(
        "value",
        DataType::dictionary(DataType::Int8, DataType::UInt8).unwrap(),
        true,
        [("owner", "yggdryl")],
    )
    .unwrap();
    field.set_dictionary_options(42, true).unwrap();
    let cached = field.to_arrow_ref().unwrap();
    let transformed = field.to_scheme_compat(&Scheme::SPARK).unwrap();
    assert_eq!(transformed.name(), field.name());
    assert!(transformed.is_nullable());
    assert_eq!(transformed.get_metadata("owner"), Some("yggdryl"));
    assert_eq!(transformed.data_type(), &DataType::Int16);
    assert_eq!(transformed.dictionary_id(), None);
    assert_eq!(transformed.dictionary_is_ordered(), None);
    assert!(!Arc::ptr_eq(&cached, &transformed.to_arrow_ref().unwrap()));
}

#[test]
fn spark_rejects_nested_extension_storage_before_rewriting() {
    let child = Field::from_parts(
        "item",
        DataType::UInt8,
        true,
        [("ARROW:extension:name", "example.byte")],
    )
    .unwrap();
    let source = DataType::list(child);
    let error = source
        .to_scheme_compat(&Scheme::SPARK)
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[].item"), "{error}");
    assert!(error.contains("extension storage"), "{error}");
}

#[test]
fn spark_rejects_both_run_end_extension_children_at_exact_paths() {
    for (extension_on_run_ends, expected_path) in
        [(true, "$.run_ends"), (false, "$.run_end_values")]
    {
        let run_ends = if extension_on_run_ends {
            Field::from_parts(
                "run_ends",
                DataType::Int32,
                false,
                [("ARROW:extension:name", "example.run-ends")],
            )
            .unwrap()
        } else {
            Field::new("run_ends", DataType::Int32, false)
        };
        let values = if extension_on_run_ends {
            Field::new("values", DataType::Utf8View, true)
        } else {
            Field::from_parts(
                "values",
                DataType::Utf8View,
                true,
                [("ARROW:extension:metadata", "example-values")],
            )
            .unwrap()
        };
        let encoded = DataType::run_end_encoded(run_ends, values).unwrap();
        let error = encoded
            .to_scheme_compat(&Scheme::SPARK)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected_path), "{error}");
        assert!(error.contains("extension storage"), "{error}");
    }
}

#[test]
fn iceberg_widens_everything_outside_its_closed_primitive_vocabulary() {
    // Iceberg has no unsigned integer and no narrow integer, so each one widens
    // to the signed primitive that holds every value it can carry.
    let widened = vec![
        (DataType::Int8, DataType::Int32),
        (DataType::Int16, DataType::Int32),
        (DataType::UInt8, DataType::Int32),
        (DataType::UInt16, DataType::Int32),
        (DataType::UInt32, DataType::Int64),
        (DataType::UInt64, DataType::decimal128(20, 0).unwrap()),
        (DataType::Float16, DataType::Float32),
        (DataType::LargeBinary, DataType::Binary),
        (DataType::BinaryView, DataType::Binary),
        (DataType::LargeUtf8, DataType::Utf8),
        (DataType::Utf8View, DataType::Utf8),
        (
            DataType::decimal32(7, 2).unwrap(),
            DataType::decimal128(7, 2).unwrap(),
        ),
        (
            DataType::decimal64(12, 2).unwrap(),
            DataType::decimal128(12, 2).unwrap(),
        ),
    ];
    for (source, expected) in widened {
        assert_eq!(
            source.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
            expected,
            "unexpected Iceberg projection for {source:?}"
        );
    }

    // The primitive vocabulary itself passes through untouched: `unknown`,
    // `boolean`, `int`, `long`, `float`, `double`, `date`, `time`, both
    // timestamp resolutions, `string`, `binary`, `fixed[n]` - which is also how
    // a uuid is stored - and a decimal already at the interchange width.
    for kept in [
        DataType::Null,
        DataType::Boolean,
        DataType::Int32,
        DataType::Int64,
        DataType::Float32,
        DataType::Float64,
        DataType::Date32,
        DataType::Binary,
        DataType::Utf8,
        DataType::fixed_size_binary(16).unwrap(),
        DataType::fixed_size_binary(8).unwrap(),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Timestamp(TimeUnit::Microsecond, Some(Timezone::UTC)),
        DataType::Timestamp(TimeUnit::Nanosecond, None),
        DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        DataType::decimal128(38, 9).unwrap(),
    ] {
        assert_eq!(
            kept.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
            kept,
            "{kept:?}"
        );
    }
}

#[test]
fn iceberg_refusals_carry_a_path_and_name_the_expectation_and_the_actual() {
    let cases = vec![
        (
            DataType::Timestamp(TimeUnit::Second, None),
            vec!["expected timestamp of us or ns", "got s", "value cast"],
        ),
        (
            DataType::Time32(TimeUnit::Millisecond),
            vec!["expected time-of-day of us", "got ms"],
        ),
        (
            DataType::Time64(TimeUnit::Nanosecond),
            vec!["expected time-of-day of us", "got ns"],
        ),
        (
            DataType::Date64,
            vec!["date64 milliseconds", "Iceberg date32 days"],
        ),
        (
            DataType::Duration(TimeUnit::Microsecond),
            vec!["no elapsed-time type", "got duration(us)"],
        ),
        (
            DataType::Interval(TimeUnit::MonthDayNano),
            vec!["no calendar interval type", "got interval(month_day_nano)"],
        ),
        (
            DataType::decimal256(39, 0).unwrap(),
            vec!["decimal256(39, 0)", "limited to 38"],
        ),
        (
            DataType::Decimal128 {
                precision: 10,
                scale: -2,
            },
            vec!["expected a non-negative decimal scale, got -2", "Iceberg"],
        ),
    ];
    for (rejected, fragments) in cases {
        let source =
            DataType::from_fields([Field::new("created", rejected.clone(), true)]).unwrap();
        let error = source.to_scheme_compat(&Scheme::ICEBERG).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("$.created"), "{rejected:?}: {message}");
        for fragment in fragments {
            assert!(message.contains(fragment), "{rejected:?}: {message}");
        }
        assert!(matches!(
            error,
            Error::InvalidDataType {
                kind: "IcebergCompatibility",
                ..
            }
        ));
    }
}

#[test]
fn iceberg_recurses_through_nested_layouts_and_declares_union_and_fixed_size_list() {
    let source = DataType::from_fields([
        Field::new("id", DataType::UInt16, false),
        Field::new(
            "tags",
            DataType::large_list(Field::new("item", DataType::Utf8View, true)),
            false,
        ),
        Field::new(
            "nested",
            DataType::from_fields([Field::new("half", DataType::Float16, true)]).unwrap(),
            true,
        ),
        // Iceberg has a first-class map, so it recurses rather than refusing.
        Field::new(
            "labels",
            DataType::map_of(DataType::Utf8View, DataType::UInt8, true).unwrap(),
            true,
        ),
    ])
    .unwrap();
    let transformed = source.to_scheme_compat(&Scheme::ICEBERG).unwrap();
    let fields = transformed.as_fields().unwrap();

    assert_eq!(fields[0].data_type(), &DataType::Int32);
    assert!(!fields[0].is_nullable());
    let DataType::List(item) = fields[1].data_type() else {
        panic!("expected a normalized list");
    };
    assert_eq!(item.data_type(), &DataType::Utf8);
    assert!(item.is_nullable());
    let nested = fields[2].data_type().as_fields().unwrap();
    assert_eq!(nested[0].name(), "half");
    assert_eq!(nested[0].data_type(), &DataType::Float32);
    let DataType::Map(map) = fields[3].data_type() else {
        panic!("expected a retained map");
    };
    assert!(map.keys_sorted());
    let entries = map.entries().data_type().as_fields().unwrap();
    assert_eq!(entries[0].data_type(), &DataType::Utf8);
    assert_eq!(entries[1].data_type(), &DataType::Int32);

    // A fixed-size list has no Iceberg equivalent, so it degrades to a list.
    let fixed = DataType::fixed_size_list(Field::new("item", DataType::Int32, false), 3).unwrap();
    assert_eq!(
        fixed.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
        DataType::list(Field::new("item", DataType::Int32, false))
    );

    // A union has none, and says so where it is.
    let union = DataType::from_fields([Field::new(
        "choice",
        DataType::union(
            [(1, Field::new("value", DataType::Int32, false))],
            UnionMode::Dense,
        )
        .unwrap(),
        true,
    )])
    .unwrap();
    let message = union
        .to_scheme_compat(&Scheme::ICEBERG)
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.choice"), "{message}");
    assert!(
        message.contains("Iceberg has no conservative tagged-union schema equivalent"),
        "{message}"
    );
}

#[test]
fn iceberg_passes_first_class_geospatial_identity_and_still_rejects_foreign_extensions() {
    // The three extension-typed variants are first class: Iceberg v3 spells
    // them itself, so they pass unchanged.
    let geometry = Field::new("shape", DataType::geometry(None).unwrap(), true);
    assert_eq!(
        geometry.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
        geometry
    );
    let variant = Field::new("payload", DataType::variant(), true);
    assert_eq!(variant.to_scheme_compat(&Scheme::ICEBERG).unwrap(), variant);

    // An imported geometry no longer carries its extension keys - they are
    // stripped as transport - so nothing trips the extension-storage rule.
    let imported = Field::from_arrow(&geometry.to_arrow().unwrap()).unwrap();
    assert!(!imported.has_metadata("ARROW:extension:name"));
    assert_eq!(
        imported.to_scheme_compat(&Scheme::ICEBERG).unwrap(),
        geometry
    );

    // A foreign extension is still rejected rather than relabeled.
    let foreign = Field::from_parts(
        "blob",
        DataType::LargeBinary,
        true,
        [("ARROW:extension:name", "someorg.blob")],
    )
    .unwrap();
    let refused = foreign
        .to_scheme_compat(&Scheme::ICEBERG)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("extension storage"), "{refused}");
}
