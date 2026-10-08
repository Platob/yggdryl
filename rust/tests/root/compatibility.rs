//! `rust/src/compatibility.rs`.

use std::io::Cursor;
use std::sync::Arc;

use yggdryl::{
    DataType, DataTypeId, Error, Field, Scheme, StructType, TimeUnit, Timezone, UnionMode,
};

#[test]
fn arrow_is_a_cache_preserving_validated_noop() {
    let field = Field::from_parts(
        "value",
        DataType::from(
            StructType::from_fields([Field::new("child", DataType::utf8(), true)]).unwrap(),
        ),
        false,
        [("owner", "yggdryl")],
    )
    .unwrap();
    let cached = Arc::new(field.clone().into_arrow_field().unwrap());
    let field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
    let compatible = field.clone().into_scheme_compat(&Scheme::ARROW).unwrap();

    assert_eq!(compatible, field);
    assert!(Arc::ptr_eq(
        &cached,
        &compatible.into_arrow_field_ref().unwrap()
    ));
}

#[test]
fn spark_applies_only_the_conservative_recursive_matrix() {
    let source = StructType::from_fields([
        Field::new("small", DataType::UInt8, false),
        Field::new("wide", DataType::UInt64, true),
        Field::new(
            "items",
            DataType::large_serie(Field::new("item", DataType::utf8_view(), true)),
            false,
        ),
        Field::new(
            "encoded",
            DataType::dictionary(DataType::Int8, DataType::large_binary()).unwrap(),
            false,
        ),
    ])
    .map(DataType::from)
    .unwrap();
    let transformed = source.into_scheme_compat(&Scheme::SPARK).unwrap();
    let fields = transformed.as_fields().unwrap();
    assert_eq!(fields[0].dtype(), &DataType::Int16);
    assert_eq!(fields[1].dtype(), &DataType::decimal128(20, 0).unwrap());
    let DataType::Serie(item) = fields[2].dtype() else {
        panic!("expected normalized serie");
    };
    assert_eq!(item.dtype(), &DataType::utf8());
    assert!(item.is_nullable());
    assert_eq!(fields[3].dtype(), &DataType::binary());
    assert!(fields[1].is_nullable());
}

#[test]
fn spark_physical_rewrite_table_covers_offset_numeric_and_decimal_families() {
    let cases = vec![
        (DataType::UInt16, DataType::Int32),
        (DataType::UInt32, DataType::Int64),
        (DataType::Float16, DataType::Float32),
        (DataType::fixed_binary(8).unwrap(), DataType::binary()),
        (DataType::large_binary(), DataType::binary()),
        (DataType::binary_view(), DataType::binary()),
        (DataType::large_utf8(), DataType::utf8()),
        (DataType::utf8_view(), DataType::utf8()),
        (
            DataType::serie_view(Field::new("item", DataType::UInt16, true)),
            DataType::serie(Field::new("item", DataType::Int32, true)),
        ),
        (
            DataType::fixed_size_serie(Field::new("item", DataType::Int32, false), 4).unwrap(),
            DataType::serie(Field::new("item", DataType::Int32, false)),
        ),
        (
            DataType::large_serie_view(Field::new("item", DataType::utf8_view(), true)),
            DataType::serie(Field::new("item", DataType::utf8(), true)),
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
            source.clone().into_scheme_compat(&Scheme::SPARK).unwrap(),
            expected,
            "unexpected Spark projection for {source:?}"
        );
    }
}

#[test]
fn spark_errors_are_path_aware_and_extension_rewrites_are_atomic() {
    let source = StructType::from_fields([Field::new(
        "a.b",
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
        false,
    )])
    .map(DataType::from)
    .unwrap();
    let error = source
        .into_scheme_compat(&Scheme::SPARK)
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
    let error = extension
        .clone()
        .into_scheme_compat(&Scheme::SPARK)
        .unwrap_err();
    assert!(matches!(error, Error::InvalidDataType { .. }));
    assert_eq!(extension, before);

    let no_op_extension = Field::from_parts(
        "value",
        DataType::utf8(),
        true,
        [("ARROW:extension:name", "example.text")],
    )
    .unwrap();
    assert_eq!(
        no_op_extension
            .clone()
            .into_scheme_compat(&Scheme::SPARK)
            .unwrap(),
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
    let error = DataType::Int32.into_scheme_compat(&duckdb).unwrap_err();
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
        .into_scheme_compat(&duckdb)
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
fn polars_keeps_unsigned_integers_and_fixed_size_series() {
    // Spark widens unsigned integers; Polars has them natively.
    assert_eq!(
        DataType::UInt32
            .into_scheme_compat(&Scheme::POLARS)
            .unwrap(),
        DataType::UInt32
    );
    assert_eq!(
        DataType::UInt32.into_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::Int64
    );

    // Polars `Array` keeps the fixed-length layout; Spark degrades to a serie.
    let fixed = DataType::fixed_size_serie(Field::new("item", DataType::Int32, false), 3).unwrap();
    assert_eq!(
        fixed.clone().into_scheme_compat(&Scheme::POLARS).unwrap(),
        fixed
    );
    assert_eq!(
        fixed.into_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::serie(Field::new("item", DataType::Int32, false))
    );
}

#[test]
fn polars_and_pandas_reject_maps_with_a_named_alternative() {
    let map = DataType::map(
        Field::new(
            "entries",
            StructType::from_fields(vec![
                Field::new("key", DataType::utf8(), false),
                Field::new("value", DataType::Int64, true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        ),
        false,
    )
    .unwrap();

    // Spark has a first-class map and keeps it.
    assert!(map.clone().into_scheme_compat(&Scheme::SPARK).is_ok());

    for target in [Scheme::POLARS, Scheme::PANDAS] {
        let message = map
            .clone()
            .into_scheme_compat(&target)
            .unwrap_err()
            .to_string();
        assert!(message.contains("no first-class map type"), "{message}");
        assert!(message.contains("key/value structs"), "{message}");
    }
}

#[test]
fn temporal_resolution_errors_name_the_expected_and_actual_unit() {
    let nanosecond = DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::NAIVE,
    };

    // pandas is nanosecond-native, Spark is microsecond-native.
    assert_eq!(
        nanosecond
            .clone()
            .into_scheme_compat(&Scheme::PANDAS)
            .unwrap(),
        nanosecond
    );
    let message = nanosecond
        .into_scheme_compat(&Scheme::SPARK)
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected timestamp of us"), "{message}");
    assert!(message.contains("got ns"), "{message}");
    assert!(message.contains("value cast"), "{message}");

    let second = DataType::DateTime64 {
        unit: TimeUnit::Second,
        timezone: Timezone::NAIVE,
    };
    let polars_message = second
        .into_scheme_compat(&Scheme::POLARS)
        .unwrap_err()
        .to_string();
    assert!(polars_message.contains("got s"), "{polars_message}");
    assert!(polars_message.contains("ms, us, or ns"), "{polars_message}");
}

#[test]
fn every_target_reports_a_path_for_a_nested_failure() {
    let nested = StructType::from_fields(vec![Field::new(
        "outer",
        DataType::serie(Field::new(
            "item",
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
            true,
        )),
        true,
    )])
    .map(DataType::from)
    .unwrap();

    for target in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
        let message = nested
            .clone()
            .into_scheme_compat(&target)
            .unwrap_err()
            .to_string();
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
        let message = negative
            .clone()
            .into_scheme_compat(&target)
            .unwrap_err()
            .to_string();
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
        nested = DataType::serie(Field::new("item", nested, false));
    }
    for (scheme, expected_kind) in [
        (Scheme::ARROW, "ArrowCompatibility"),
        (Scheme::SPARK, "SparkCompatibility"),
        (Scheme::POLARS, "PolarsCompatibility"),
        (Scheme::PANDAS, "PandasCompatibility"),
        (Scheme::ICEBERG, "IcebergCompatibility"),
    ] {
        let error = nested.clone().into_scheme_compat(&scheme).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidDataType { kind, .. } if kind == expected_kind
        ));
    }
}

#[test]
fn spark_temporal_decimal_and_union_boundaries_are_explicit() {
    for accepted in [
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        },
        DataType::Duration32(TimeUnit::Microsecond),
        DataType::Duration64(TimeUnit::Microsecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::decimal128(38, 0).unwrap(),
    ] {
        assert_eq!(
            accepted.clone().into_scheme_compat(&Scheme::SPARK).unwrap(),
            accepted
        );
    }
    for rejected in [
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
        DataType::date64(),
        DataType::Time32(TimeUnit::Second),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration32(TimeUnit::Nanosecond),
        DataType::Duration64(TimeUnit::Nanosecond),
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
            rejected.clone().into_scheme_compat(&Scheme::SPARK).is_err(),
            "{rejected:?}"
        );
    }
}

#[test]
fn spark_recurses_through_map_dictionary_and_run_end_layouts() {
    let map = DataType::map_of(DataType::utf8_view(), DataType::UInt8, true).unwrap();
    let transformed = map.into_scheme_compat(&Scheme::SPARK).unwrap();
    let Some(map) = (transformed).as_mapping() else {
        panic!("expected map");
    };
    assert!(map.keys_sorted());
    let fields = map.entries().dtype().as_fields().unwrap();
    assert_eq!(fields[0].dtype(), &DataType::utf8());
    assert_eq!(fields[1].dtype(), &DataType::Int16);

    let dictionary = DataType::dictionary(
        DataType::Int16,
        DataType::serie(Field::new("item", DataType::UInt16, true)),
    )
    .unwrap();
    let transformed = dictionary.into_scheme_compat(&Scheme::SPARK).unwrap();
    let DataType::Serie(item) = transformed else {
        panic!("expected logical dictionary serie");
    };
    assert_eq!(item.dtype(), &DataType::Int32);

    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::utf8_view(), true),
    )
    .unwrap();
    assert_eq!(
        encoded.into_scheme_compat(&Scheme::SPARK).unwrap(),
        DataType::utf8()
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
    let cached = field.clone().into_arrow_field_ref().unwrap();
    let transformed = field.clone().into_scheme_compat(&Scheme::SPARK).unwrap();
    assert_eq!(transformed.name(), field.name());
    assert!(transformed.is_nullable());
    assert_eq!(transformed.get_metadata("owner"), Some("yggdryl"));
    assert_eq!(transformed.dtype(), &DataType::Int16);
    assert_eq!(transformed.dictionary_id(), None);
    assert_eq!(transformed.dictionary_is_ordered(), None);
    assert!(!Arc::ptr_eq(
        &cached,
        &transformed.into_arrow_field_ref().unwrap()
    ));
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
    let source = DataType::serie(child);
    let error = source
        .into_scheme_compat(&Scheme::SPARK)
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
            Field::new("values", DataType::utf8_view(), true)
        } else {
            Field::from_parts(
                "values",
                DataType::utf8_view(),
                true,
                [("ARROW:extension:metadata", "example-values")],
            )
            .unwrap()
        };
        let encoded = DataType::run_end_encoded(run_ends, values).unwrap();
        let error = encoded
            .into_scheme_compat(&Scheme::SPARK)
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
        (DataType::large_binary(), DataType::binary()),
        (DataType::binary_view(), DataType::binary()),
        (DataType::large_utf8(), DataType::utf8()),
        (DataType::utf8_view(), DataType::utf8()),
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
            source.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap(),
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
        DataType::date32(),
        DataType::binary(),
        DataType::utf8(),
        DataType::fixed_binary(16).unwrap(),
        DataType::fixed_binary(8).unwrap(),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        },
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        },
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
        DataType::decimal128(38, 9).unwrap(),
    ] {
        assert_eq!(
            kept.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap(),
            kept,
            "{kept:?}"
        );
    }
}

#[test]
fn iceberg_refusals_carry_a_path_and_name_the_expectation_and_the_actual() {
    let cases = vec![
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
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
            DataType::date64(),
            vec!["date64 milliseconds", "Iceberg date32 days"],
        ),
        (
            DataType::Duration32(TimeUnit::Microsecond),
            vec!["no elapsed-time type", "got duration32(us)"],
        ),
        (
            DataType::Duration64(TimeUnit::Microsecond),
            vec!["no elapsed-time type", "got duration64(us)"],
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
        let source = DataType::from(
            StructType::from_fields([Field::new("created", rejected.clone(), true)]).unwrap(),
        );
        let error = source.into_scheme_compat(&Scheme::ICEBERG).unwrap_err();
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
fn iceberg_recurses_through_nested_layouts_and_declares_union_and_fixed_size_serie() {
    let source = StructType::from_fields([
        Field::new("id", DataType::UInt16, false),
        Field::new(
            "tags",
            DataType::large_serie(Field::new("item", DataType::utf8_view(), true)),
            false,
        ),
        Field::new(
            "nested",
            DataType::from(
                StructType::from_fields([Field::new("half", DataType::Float16, true)]).unwrap(),
            ),
            true,
        ),
        // Iceberg has a first-class map, so it recurses rather than refusing.
        Field::new(
            "labels",
            DataType::map_of(DataType::utf8_view(), DataType::UInt8, true).unwrap(),
            true,
        ),
    ])
    .map(DataType::from)
    .unwrap();
    let transformed = source.into_scheme_compat(&Scheme::ICEBERG).unwrap();
    let fields = transformed.as_fields().unwrap();

    assert_eq!(fields[0].dtype(), &DataType::Int32);
    assert!(!fields[0].is_nullable());
    let DataType::Serie(item) = fields[1].dtype() else {
        panic!("expected a normalized serie");
    };
    assert_eq!(item.dtype(), &DataType::utf8());
    assert!(item.is_nullable());
    let nested = fields[2].dtype().as_fields().unwrap();
    assert_eq!(nested[0].name(), "half");
    assert_eq!(nested[0].dtype(), &DataType::Float32);
    let Some(map) = (fields[3].dtype()).as_mapping() else {
        panic!("expected a retained map");
    };
    assert!(map.keys_sorted());
    let entries = map.entries().dtype().as_fields().unwrap();
    assert_eq!(entries[0].dtype(), &DataType::utf8());
    assert_eq!(entries[1].dtype(), &DataType::Int32);

    // A fixed-size serie has no Iceberg equivalent, so it degrades to a serie.
    let fixed = DataType::fixed_size_serie(Field::new("item", DataType::Int32, false), 3).unwrap();
    assert_eq!(
        fixed.into_scheme_compat(&Scheme::ICEBERG).unwrap(),
        DataType::serie(Field::new("item", DataType::Int32, false))
    );

    // A union has none, and says so where it is.
    let union = StructType::from_fields([Field::new(
        "choice",
        DataType::union(
            [(1, Field::new("value", DataType::Int32, false))],
            UnionMode::Dense,
        )
        .unwrap(),
        true,
    )])
    .map(DataType::from)
    .unwrap();
    let message = union
        .into_scheme_compat(&Scheme::ICEBERG)
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
    // Variant and the geospatial pair are first class in Iceberg v3, so they
    // pass unchanged; an ASCII width is Iceberg text (see
    // compatibility_reads_every_width_as_utf8).
    let geometry = Field::new("shape", DataType::geometry(None).unwrap(), true);
    assert_eq!(
        geometry
            .clone()
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
        geometry
    );
    let variant = Field::new("payload", DataType::variant(), true);
    assert_eq!(
        variant
            .clone()
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
        variant
    );

    // An imported geometry no longer carries its extension keys - they are
    // stripped as transport - so nothing trips the extension-storage rule.
    let imported = Field::from_arrow_field(&geometry.clone().into_arrow_field().unwrap()).unwrap();
    assert!(!imported.has_metadata("ARROW:extension:name"));
    assert_eq!(
        imported.into_scheme_compat(&Scheme::ICEBERG).unwrap(),
        geometry
    );

    // A foreign extension is still rejected rather than relabeled.
    let foreign = Field::from_parts(
        "blob",
        DataType::large_binary(),
        true,
        [("ARROW:extension:name", "someorg.blob")],
    )
    .unwrap();
    let refused = foreign
        .into_scheme_compat(&Scheme::ICEBERG)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("extension storage"), "{refused}");
}

#[test]
fn every_scalar_leaf_has_an_answer_for_every_target() {
    // Seven leaves - Side, State, TimeInForce, Bbg, Timezone, MimeType
    // and MediaType - were absent from all four per-target matches, so each
    // fell through to the container arm and answered "expected a scalar
    // datatype, got side; this container is handled by the generic walker".
    // Twenty-eight wrong answers, and nothing caught them because a missing
    // arm is not a compile error.
    //
    // This walks every parameter-free identifier instead of the seven, so the
    // next leaf added without a compatibility arm fails here.
    for id in DataTypeId::all() {
        if id.is_parameterized() {
            continue;
        }
        let Ok(dtype) = DataType::from_str(id.as_str()) else {
            continue; // an identifier with no standalone datatype spelling
        };
        for scheme in [
            &Scheme::SPARK,
            &Scheme::POLARS,
            &Scheme::PANDAS,
            &Scheme::ICEBERG,
        ] {
            if let Err(error) = dtype.clone().into_scheme_compat(scheme) {
                let reason = error.to_string();
                assert!(
                    !reason.contains("handled by the generic walker"),
                    "{} has no arm for {scheme}: {reason}",
                    id.as_str()
                );
            }
        }
    }
}

/// Every foreign engine reads an enum member as the `int32` code of its
/// leaf and a registered code as the text it stores, so the two families
/// are answered once above the four matrices - and Arrow keeps both.
#[test]
fn every_foreign_engine_reads_an_enum_as_its_int32_code_and_a_code_as_its_text() {
    let mut enums = 0;
    let mut codes = 0;
    for id in DataTypeId::all() {
        if id.is_parameterized() {
            continue;
        }
        let Ok(dtype) = DataType::from_str(id.as_str()) else {
            continue;
        };
        let expected = if dtype.is_enum() {
            enums += 1;
            DataType::Int32
        } else if dtype.is_code() {
            codes += 1;
            DataType::utf8()
        } else {
            continue;
        };
        for scheme in [
            &Scheme::SPARK,
            &Scheme::POLARS,
            &Scheme::PANDAS,
            &Scheme::ICEBERG,
        ] {
            assert_eq!(
                dtype.clone().into_scheme_compat(scheme).unwrap(),
                expected,
                "{dtype} under {scheme}"
            );
        }
        assert_eq!(
            dtype.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
            dtype
        );
    }
    assert!(enums >= 5, "{enums} enums seen");
    assert!(codes >= 12, "{codes} codes seen");
}

/// What the Iceberg widening lands on is a type the Iceberg writer spells:
/// the compatibility matrix is the cast, `PrimitiveType::from_dtype` the
/// stored type, and every answer of the first is a question the second
/// answers - the nested and geospatial kinds aside, which the writer does
/// not yet spell.
#[test]
fn every_iceberg_widening_lands_on_a_type_the_iceberg_writer_spells() {
    use yggdryl::DataTypeKind;
    use yggdryl::iceberg::PrimitiveType;
    let mut samples: Vec<DataType> = DataTypeId::all()
        .iter()
        .filter(|id| !id.is_parameterized())
        .filter_map(|id| DataType::from_str(id.as_str()).ok())
        .collect();
    samples.extend([
        DataType::decimal32(7, 2).unwrap(),
        DataType::decimal64(12, 2).unwrap(),
        DataType::decimal128(38, 9).unwrap(),
        DataType::sized_utf8(8).unwrap(),
        DataType::fixed_cp1252(4).unwrap(),
        DataType::LargeBinary,
        DataType::Decimal,
    ]);
    let mut seen = 0;
    for dtype in samples {
        if matches!(
            dtype.kind(),
            DataTypeKind::Nested | DataTypeKind::Geospatial
        ) {
            continue;
        }
        let Ok(widened) = dtype.clone().into_scheme_compat(&Scheme::ICEBERG) else {
            continue;
        };
        seen += 1;
        assert!(
            PrimitiveType::from_dtype(&widened).is_ok(),
            "{dtype} widened to {widened}, which Iceberg does not spell"
        );
    }
    assert!(seen > 20, "{seen} widenings seen");
    // An unsigned column stating its bits lands on the signed integer of its
    // width, which the writer spells too.
    for unsigned in [
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
    ] {
        let exchanged = bits_column(unsigned.clone())
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap();
        assert!(
            PrimitiveType::from_dtype(exchanged.dtype()).is_ok(),
            "{unsigned} stating bits exchanged as {}, which Iceberg does not spell",
            exchanged.dtype()
        );
    }
}

/// A column of `dtype` stating its integers are bits.
fn bits_column(dtype: DataType) -> Field {
    let mut field = dtype.required_field("digest");
    field
        .as_field_properties_mut()
        .set_representation(yggdryl::Representation::Bits)
        .unwrap();
    field
}

/// An unsigned column stating bits is exchanged as the signed integer of
/// its width wherever the target names that width, keeps the declaration,
/// and is otherwise rewritten as any column; nothing stating it moves.
#[test]
fn an_unsigned_column_stating_bits_takes_the_signed_integer_of_its_width() {
    use yggdryl::Representation;

    let decimal = DataType::decimal128(20, 0).unwrap();
    for (scheme, expected) in [
        (
            Scheme::SPARK,
            [
                DataType::Int8,
                DataType::Int16,
                DataType::Int32,
                DataType::Int64,
            ],
        ),
        (
            Scheme::ICEBERG,
            [
                DataType::Int32,
                DataType::Int32,
                DataType::Int32,
                DataType::Int64,
            ],
        ),
        (
            Scheme::POLARS,
            [
                DataType::UInt8,
                DataType::UInt16,
                DataType::UInt32,
                DataType::UInt64,
            ],
        ),
        (
            Scheme::PANDAS,
            [
                DataType::UInt8,
                DataType::UInt16,
                DataType::UInt32,
                DataType::UInt64,
            ],
        ),
        (
            Scheme::ARROW,
            [
                DataType::UInt8,
                DataType::UInt16,
                DataType::UInt32,
                DataType::UInt64,
            ],
        ),
    ] {
        for (unsigned, expected) in [
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
        ]
        .into_iter()
        .zip(expected)
        {
            let exchanged = bits_column(unsigned.clone())
                .into_scheme_compat(&scheme)
                .unwrap();
            assert_eq!(exchanged.dtype(), &expected, "{unsigned} under {scheme}");
            assert_eq!(
                exchanged.as_field_properties().representation(),
                Representation::Bits,
                "{unsigned} under {scheme}"
            );
        }
    }

    // A column stating nothing still widens, and a bare datatype states
    // nothing.
    for scheme in [Scheme::SPARK, Scheme::ICEBERG] {
        assert_eq!(
            DataType::UInt64
                .required_field("count")
                .into_scheme_compat(&scheme)
                .unwrap()
                .dtype(),
            &decimal
        );
        assert_eq!(
            DataType::UInt64.into_scheme_compat(&scheme).unwrap(),
            decimal
        );
    }

    // The rule holds at any depth: a struct's child, a serie's item and a
    // map's value.
    let row = DataType::from(
        StructType::from_fields([
            bits_column(DataType::UInt64),
            DataType::UInt64.required_field("count"),
            DataType::serie(bits_column(DataType::UInt64)).nullable_field("items"),
            DataType::map(
                DataType::from(
                    StructType::from_fields([
                        DataType::utf8().required_field("key"),
                        bits_column(DataType::UInt64).with_name("value"),
                    ])
                    .unwrap(),
                )
                .required_field("entries"),
                false,
            )
            .unwrap()
            .nullable_field("mapped"),
        ])
        .unwrap(),
    )
    .required_field("row");
    let exchanged = row.into_scheme_compat(&Scheme::ICEBERG).unwrap();
    let fields = exchanged.fields();
    assert_eq!(fields[0].dtype(), &DataType::Int64);
    assert_eq!(fields[1].dtype(), &decimal);
    let item = fields[2].dtype().get_field(0).unwrap();
    assert_eq!(item.dtype(), &DataType::Int64);
    assert_eq!(
        item.as_field_properties().representation(),
        Representation::Bits
    );
    let entries = fields[3].dtype().get_field(0).unwrap();
    let value = entries.dtype().get_field(1).unwrap();
    assert_eq!(value.dtype(), &DataType::Int64);
    assert_eq!(
        value.as_field_properties().representation(),
        Representation::Bits
    );
}
