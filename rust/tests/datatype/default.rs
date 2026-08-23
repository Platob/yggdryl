use yggdryl::{DataType, Field, TimeUnit, Timezone, UnionMode, Value};

fn all_variants() -> Vec<DataType> {
    let item = || Field::new("item", DataType::Int32, true);
    vec![
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
        DataType::Timestamp(TimeUnit::Microsecond, Some(Timezone::UTC)),
        DataType::Date32,
        DataType::Date64,
        DataType::Time32(TimeUnit::Millisecond),
        DataType::Time64(TimeUnit::Nanosecond),
        DataType::Duration32(TimeUnit::Second),
        DataType::Duration64(TimeUnit::Second),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::Binary,
        DataType::fixed_size_binary(3).unwrap(),
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::list(item()),
        DataType::list_view(item()),
        DataType::fixed_size_list(item(), 2).unwrap(),
        DataType::large_list(item()),
        DataType::large_list_view(item()),
        DataType::from_fields([
            Field::new("required", DataType::Int32, false),
            Field::new("optional", DataType::Utf8, true),
        ])
        .unwrap(),
        DataType::union(
            [
                (2, Field::new("nothing", DataType::Null, true)),
                (7, Field::new("number", DataType::Int32, false)),
            ],
            UnionMode::Dense,
        )
        .unwrap(),
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        DataType::decimal32(7, 2).unwrap(),
        DataType::decimal64(12, 3).unwrap(),
        DataType::decimal128(30, 4).unwrap(),
        DataType::decimal256(50, 5).unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int32, true).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
    ]
}

#[test]
fn every_datatype_variant_has_a_bounded_valid_default() {
    for data_type in all_variants() {
        let value = data_type
            .default_value()
            .unwrap_or_else(|error| panic!("{} default failed: {error}", data_type.kind()));
        assert!(
            data_type
                .is_default_value(&value)
                .unwrap_or_else(|error| panic!(
                    "{} default match failed: {error}",
                    data_type.kind()
                )),
            "{} did not recognize its canonical default",
            data_type.kind()
        );
        let nullable = matches!(data_type, DataType::Null);
        let field = Field::new("value", data_type.clone(), nullable);
        assert_eq!(
            field.default_value().unwrap_or_else(|error| {
                panic!("{} Field default failed: {error}", data_type.kind())
            }),
            value
        );
        let root = Field::new("Root", DataType::from_fields([field]).unwrap(), false);
        let row = Value::from_sequence([value]);
        root.validate_value(&row).unwrap_or_else(|error| {
            panic!("{} default did not validate: {error}", data_type.kind())
        });
    }
}

#[test]
fn default_matching_is_allocation_free_for_wide_values_and_exact_for_unions() {
    let wide = DataType::FixedSizeBinary(64 * 1024 * 1024);
    assert!(!wide.is_default_value(&Value::Null).unwrap());
    assert!(!wide.is_default_value(&Value::from(vec![0_u8; 3])).unwrap());

    let union = DataType::union(
        [
            (2, Field::new("nothing", DataType::Null, false)),
            (7, Field::new("number", DataType::Int32, false)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let expected = union.default_value().unwrap();
    assert!(union.is_default_value(&expected).unwrap());
    assert!(union.default_union_type_id().unwrap() == Some(7));
    assert!(
        !union
            .is_default_value(&Value::from_sequence([Value::I64(2), Value::I64(0)]))
            .unwrap()
    );
    assert!(
        !union
            .is_default_value(&Value::from_sequence([Value::I64(7), Value::I64(1)]))
            .unwrap()
    );

    let fatal = DataType::FixedSizeBinary(64 * 1024 * 1024 + 1);
    assert!(fatal.is_default_value(&Value::Null).is_err());
}

#[test]
fn nested_defaults_respect_child_field_nullability() {
    let structure = DataType::from_fields([
        Field::new("required", DataType::Int32, false),
        Field::new("optional", DataType::Utf8, true),
    ])
    .unwrap();
    assert_eq!(
        structure.default_value().unwrap().as_sequence().unwrap(),
        &[Value::I64(0), Value::Null]
    );

    let fixed = DataType::fixed_size_list(Field::new("item", DataType::Int32, true), 3).unwrap();
    assert_eq!(
        fixed.default_value().unwrap().as_sequence().unwrap(),
        &[Value::Null, Value::Null, Value::Null]
    );
}

#[test]
fn field_defaults_apply_physical_union_and_run_end_nulls() {
    let union = DataType::union(
        [
            (3, Field::new("required", DataType::Int32, false)),
            (9, Field::new("optional", DataType::Utf8, true)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let nullable = Field::new("choice", union.clone(), true);
    assert_eq!(
        nullable.default_value().unwrap().as_sequence().unwrap(),
        &[Value::I64(9), Value::Null]
    );
    assert_eq!(
        Field::new("choice", union, false)
            .default_value()
            .unwrap()
            .as_sequence()
            .unwrap(),
        &[Value::I64(3), Value::I64(0)]
    );

    let nullable_run = Field::new(
        "runs",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
        true,
    );
    assert_eq!(nullable_run.default_value().unwrap(), Value::Null);

    let required_values = Field::new(
        "runs",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, false),
        )
        .unwrap(),
        true,
    );
    assert!(required_values.default_value().is_err());
    assert!(
        Field::new("null", DataType::Null, false)
            .default_value()
            .is_err()
    );
}

#[test]
fn defaults_reject_invalid_or_unbounded_caller_constructed_layouts() {
    for invalid in [
        DataType::Time32(TimeUnit::Nanosecond),
        DataType::Time64(TimeUnit::Millisecond),
        DataType::Duration32(TimeUnit::DayTime),
        DataType::Duration64(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::Second),
        DataType::FixedSizeBinary(-1),
        DataType::FixedSizeList(
            std::sync::Arc::new(Field::new("item", DataType::Int32, false)),
            -1,
        ),
        DataType::Decimal32 {
            precision: 0,
            scale: 0,
        },
        DataType::Decimal64 {
            precision: 19,
            scale: 0,
        },
        DataType::Decimal128 {
            precision: 39,
            scale: 0,
        },
        DataType::Decimal256 {
            precision: 77,
            scale: 0,
        },
    ] {
        assert!(invalid.default_value().is_err(), "{invalid:?}");
    }
    let too_wide = DataType::FixedSizeBinary(64 * 1024 * 1024 + 1);
    let error = too_wide.default_value().unwrap_err().to_string();
    assert!(error.contains("byte safety limit"), "{error}");

    let mut maximum = DataType::Int32;
    for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
        maximum = DataType::list(Field::new("a.b", maximum, false));
    }
    assert!(maximum.default_value().is_ok());
    let overdeep = DataType::list(Field::new("a.b", maximum, false));
    let error = overdeep.default_value().unwrap_err().to_string();
    assert!(error.contains("hard limit"), "{error}");

    let large_child = Field::new("item", DataType::FixedSizeBinary(40 * 1024 * 1024), false);
    let multiplicative = DataType::fixed_size_list(large_child, 2).unwrap();
    let error = multiplicative.default_value().unwrap_err().to_string();
    assert!(error.contains("byte safety limit"), "{error}");
}

#[test]
fn fatal_default_limits_never_fall_back_to_nullable_nulls() {
    let oversized = DataType::FixedSizeBinary(64 * 1024 * 1024 + 1);
    let union = DataType::union(
        [(1, Field::new("oversized", oversized.clone(), true))],
        UnionMode::Dense,
    )
    .unwrap();
    let error = union.default_value().unwrap_err().to_string();
    assert!(error.contains("byte safety limit"), "{error}");

    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", oversized, true),
    )
    .unwrap();
    let error = encoded.default_value().unwrap_err().to_string();
    assert!(error.contains("byte safety limit"), "{error}");

    let fallback = DataType::union(
        [
            (1, Field::new("uninhabited", DataType::Null, false)),
            (2, Field::new("present", DataType::Int32, false)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    assert_eq!(
        fallback.default_value().unwrap(),
        Value::from_sequence([Value::I64(2), Value::I64(0)])
    );
}

#[test]
fn null_only_nested_layouts_obey_physical_field_constraints() {
    let zero = DataType::fixed_size_list(Field::new("item", DataType::Null, false), 0).unwrap();
    assert_eq!(zero.default_value().unwrap(), Value::from_sequence([]));
    let positive = DataType::fixed_size_list(Field::new("item", DataType::Null, false), 1).unwrap();
    assert!(positive.default_value().is_err());

    let required_null =
        DataType::from_fields([Field::new("nothing", DataType::Null, false)]).unwrap();
    assert!(required_null.default_value().is_err());
    let optional_null =
        DataType::from_fields([Field::new("nothing", DataType::Null, true)]).unwrap();
    assert_eq!(
        optional_null
            .default_value()
            .unwrap()
            .as_sequence()
            .unwrap(),
        &[Value::Null]
    );

    let no_nullable_branch = DataType::union(
        [(1, Field::new("number", DataType::Int32, false))],
        UnionMode::Sparse,
    )
    .unwrap();
    assert!(
        Field::new("choice", no_nullable_branch, true)
            .default_value()
            .is_err()
    );
}

#[test]
fn opaque_nested_types_reject_malformed_construction_before_defaults() {
    assert!(DataType::dictionary(DataType::Float32, DataType::Utf8).is_err());
    assert!(
        DataType::union([], UnionMode::Dense)
            .unwrap()
            .default_value()
            .is_err()
    );
    assert!(DataType::map(Field::new("entries", DataType::Utf8, false), false,).is_err());
    assert!(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::UInt32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .is_err()
    );
}
