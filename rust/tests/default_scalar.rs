//! Canonical default Arrow scalar contract tests.

use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{Array, ArrayRef, DictionaryArray, Int8Array, Int32Array, StringArray};
use yggdryl::arrow::{ArrowScalar, DefaultArrowScalar};
use yggdryl::{DataType, Field, TimeUnit, Timezone, UnionMode, Value};

fn representative_types() -> Vec<DataType> {
    let item = || Field::new("item", DataType::Int32, true);
    vec![
        DataType::Null,
        DataType::Boolean,
        DataType::Int8,
        DataType::UInt64,
        DataType::Float16,
        DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        DataType::Date32,
        DataType::Date64,
        DataType::Time32(TimeUnit::Second),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration(TimeUnit::Millisecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::Binary,
        DataType::fixed_size_binary(2).unwrap(),
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
                (1, Field::new("nothing", DataType::Null, true)),
                (4, Field::new("number", DataType::Int32, false)),
            ],
            UnionMode::Dense,
        )
        .unwrap(),
        DataType::dictionary(DataType::Int8, DataType::Utf8).unwrap(),
        DataType::decimal32(7, 2).unwrap(),
        DataType::decimal64(12, 2).unwrap(),
        DataType::decimal128(30, 2).unwrap(),
        DataType::decimal256(50, 2).unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int32, false).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
    ]
}

#[test]
fn datatype_defaults_round_trip_through_the_public_parts_constructor() {
    for data_type in representative_types() {
        let expected = data_type.default_value().unwrap();
        let scalar = data_type
            .default_arrow_scalar()
            .unwrap_or_else(|error| panic!("{} Arrow default failed: {error}", data_type.kind()));
        assert_eq!(scalar.field().name(), "value");
        assert!(!scalar.field().is_nullable());
        assert_eq!(scalar.data_type(), &data_type);
        assert_eq!(scalar.array().len(), 1);
        // The Arrow reading spells temporals and decimals with their unit,
        // zone, or scale; the canonical default recognizes both spellings.
        let read = scalar.to_value().unwrap();
        assert!(
            data_type.is_default_value(&read).unwrap(),
            "{} read {read:?} is not the default {expected:?}",
            data_type.kind()
        );

        let (field, array) = scalar.into_parts();
        let rebuilt = ArrowScalar::from_parts(field, array).unwrap();
        let read = rebuilt.into_value().unwrap();
        assert!(
            data_type.is_default_value(&read).unwrap(),
            "{} rebuilt {read:?} is not the default {expected:?}",
            data_type.kind()
        );
    }
}

#[test]
fn field_defaults_preserve_exact_identity_and_nullability() {
    let field = Field::from_parts(
        "price",
        DataType::decimal128(9, 2).unwrap(),
        true,
        [("ARROW:extension:name", "example.price")],
    )
    .unwrap();
    let scalar = field.default_arrow_scalar().unwrap();
    assert_eq!(scalar.field(), &field);
    assert!(scalar.array().is_null(0));
    let cloned = scalar.to_array();
    assert!(Arc::ptr_eq(scalar.array(), &cloned));

    assert!(
        Field::new("never", DataType::Null, false)
            .default_arrow_scalar()
            .is_err()
    );
}

#[test]
fn external_parts_reject_wrong_lengths_types_and_recursive_nullability() {
    let scalar = DataType::Int32.default_arrow_scalar().unwrap();
    assert!(
        ArrowScalar::from_parts(
            Field::new("value", DataType::Int64, false),
            scalar.to_array(),
        )
        .is_err()
    );
    assert!(
        ArrowScalar::from_parts(
            Field::new("value", DataType::Int32, false),
            scalar.array().slice(0, 0),
        )
        .is_err()
    );

    let nullable_child = Field::new("child", DataType::Int32, true)
        .default_arrow_scalar()
        .unwrap();
    assert!(
        ArrowScalar::from_parts(
            Field::new("child", DataType::Int32, false),
            nullable_child.into_array(),
        )
        .is_err()
    );
    assert!(
        ArrowScalar::from_value(Field::new("child", DataType::Int32, false), Value::Null,).is_err()
    );
}

#[test]
fn intrinsic_logical_null_wrappers_round_trip_but_arbitrary_selected_null_does_not() {
    let intrinsic_defaults = [
        DataType::dictionary(DataType::Int8, DataType::Null).unwrap(),
        DataType::union(
            [(5, Field::new("nothing", DataType::Null, true))],
            UnionMode::Dense,
        )
        .unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int16, false),
            Field::new("values", DataType::Null, true),
        )
        .unwrap(),
    ];
    for data_type in intrinsic_defaults {
        let expected = data_type.default_value().unwrap();
        let scalar = data_type.default_arrow_scalar().unwrap();
        let (field, array) = scalar.into_parts();
        let rebuilt = ArrowScalar::from_parts(field, array).unwrap();
        assert_eq!(rebuilt.into_value().unwrap(), expected);
    }

    let union = DataType::union(
        [
            (1, Field::new("present", DataType::Int32, false)),
            (9, Field::new("absent", DataType::Utf8, true)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let selected_null = Field::new("choice", union.clone(), true)
        .default_arrow_scalar()
        .unwrap();
    assert_ne!(
        selected_null.to_value().unwrap(),
        union.default_value().unwrap()
    );
    assert!(
        ArrowScalar::from_parts(
            Field::new("choice", union, false),
            selected_null.into_array(),
        )
        .is_err()
    );
}

#[test]
fn nullable_dictionary_null_keys_decode_as_native_null() {
    let data_type = DataType::dictionary(DataType::Int8, DataType::Utf8).unwrap();
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![None]),
            Arc::new(StringArray::from(vec!["not-null"])),
        )
        .unwrap(),
    );
    let scalar = ArrowScalar::from_parts(Field::new("encoded", data_type, true), array).unwrap();
    assert_eq!(scalar.to_value().unwrap(), Value::Null);
}

#[test]
fn external_parts_preflight_deep_caller_built_schemas_before_arrow_projection() {
    let mut maximum = DataType::Int32;
    for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
        maximum = DataType::list(Field::new("item", maximum, false));
    }
    let scalar = maximum.default_arrow_scalar().unwrap();
    let (field, array) = scalar.into_parts();
    ArrowScalar::from_parts(field, array).unwrap();

    let overdeep = DataType::list(Field::new("item", maximum, false));
    let unrelated: ArrayRef = Arc::new(Int32Array::from(vec![0]));
    let error = ArrowScalar::from_parts(Field::new("value", overdeep, false), unrelated)
        .unwrap_err()
        .to_string();
    assert!(error.contains("hard limit"), "{error}");
}
