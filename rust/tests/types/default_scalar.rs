//! Canonical default Arrow scalar contract tests.

use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{Array, ArrayRef, DictionaryArray, Int8Array, Int32Array, StringArray};
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::{DataType, Field, Scalar, TimeUnit, Timezone, TypedScalar, UnionMode};

fn representative_types() -> Vec<DataType> {
    let item = || Field::new("item", DataType::Int32, true);
    vec![
        DataType::Null,
        DataType::Boolean,
        DataType::Int8,
        DataType::UInt64,
        DataType::Float16,
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        },
        DataType::Date32,
        DataType::Date64,
        DataType::Time32(TimeUnit::Second),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration32(TimeUnit::Millisecond),
        DataType::Duration64(TimeUnit::Millisecond),
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
fn datatype_defaults_round_trip_through_the_public_scalar_boundary() {
    for dtype in representative_types() {
        let expected = dtype.default_value().unwrap();
        let array = dtype
            .default_arrow_array()
            .unwrap_or_else(|error| panic!("{} Arrow default failed: {error}", dtype.kind()));
        assert_eq!(array.len(), 1);
        // The default projects through a synthetic non-nullable Field, which is
        // exactly what the foreign-array importer's canonical-default exception
        // exists to accept back.
        let field = Field::new("value", dtype.clone(), false);
        // The Arrow reading spells temporals and decimals with their unit,
        // zone, or scale; the canonical default recognizes both spellings.
        let read = scalar_value(&field, array.as_ref()).unwrap();
        assert!(
            dtype.is_default_value(&read).unwrap(),
            "{} read {read:?} is not the default {expected:?}",
            dtype.kind()
        );

        // The same array decodes as a typed pairing, which re-projects to an
        // equal one-row array: the default is closed under both directions.
        let typed = TypedScalar::from_arrow_array(dtype.clone(), array.as_ref())
            .unwrap_or_else(|error| panic!("{} typed decode failed: {error}", dtype.kind()));
        assert_eq!(typed.dtype(), &dtype);
        assert!(
            dtype.is_default_value(typed.value()).unwrap(),
            "{} typed {:?} is not the default {expected:?}",
            dtype.kind(),
            typed.value()
        );
        let reprojected = typed
            .into_arrow_array()
            .unwrap_or_else(|error| panic!("{} reprojection failed: {error}", dtype.kind()));
        assert_eq!(reprojected.as_ref(), array.as_ref());
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
    let array = field.default_arrow_array().unwrap();
    assert_eq!(array.len(), 1);
    assert!(array.is_null(0));
    assert_eq!(scalar_value(&field, array.as_ref()).unwrap(), Scalar::Null);

    assert!(
        Field::new("never", DataType::Null, false)
            .default_arrow_array()
            .is_err()
    );
}

#[test]
fn foreign_arrays_reject_wrong_lengths_types_and_recursive_nullability() {
    let array = DataType::Int32.default_arrow_array().unwrap();
    assert!(scalar_value(&Field::new("value", DataType::Int64, false), array.as_ref()).is_err());
    assert!(
        scalar_value(
            &Field::new("value", DataType::Int32, false),
            array.slice(0, 0).as_ref(),
        )
        .is_err()
    );

    let nullable_child = Field::new("child", DataType::Int32, true)
        .default_arrow_array()
        .unwrap();
    assert!(
        scalar_value(
            &Field::new("child", DataType::Int32, false),
            nullable_child.as_ref(),
        )
        .is_err()
    );
    assert!(scalar_array(&Field::new("child", DataType::Int32, false), &Scalar::Null).is_err());
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
    for dtype in intrinsic_defaults {
        let expected = dtype.default_value().unwrap();
        let array = dtype.default_arrow_array().unwrap();
        // The canonical-default exception admits the logical-null default back
        // through a non-nullable Field...
        let field = Field::new("value", dtype.clone(), false);
        assert_eq!(scalar_value(&field, array.as_ref()).unwrap(), expected);
        // ...and the typed pairing projects the same null-only default out
        // through its synthetic non-nullable Field.
        let typed = TypedScalar::from_parts(dtype, expected).unwrap();
        assert_eq!(typed.into_arrow_array().unwrap().as_ref(), array.as_ref());
    }

    let union = DataType::union(
        [
            (1, Field::new("present", DataType::Int32, false)),
            (9, Field::new("absent", DataType::Utf8, true)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let nullable = Field::new("choice", union.clone(), true);
    let selected_null = nullable.default_arrow_array().unwrap();
    assert_ne!(
        scalar_value(&nullable, selected_null.as_ref()).unwrap(),
        union.default_value().unwrap()
    );
    assert!(scalar_value(&Field::new("choice", union, false), selected_null.as_ref(),).is_err());
}

#[test]
fn nullable_dictionary_null_keys_decode_as_native_null() {
    let dtype = DataType::dictionary(DataType::Int8, DataType::Utf8).unwrap();
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![None]),
            Arc::new(StringArray::from(vec!["not-null"])),
        )
        .unwrap(),
    );
    assert_eq!(
        scalar_value(&Field::new("encoded", dtype, true), array.as_ref()).unwrap(),
        Scalar::Null
    );
}

#[test]
fn foreign_arrays_preflight_deep_caller_built_schemas_before_arrow_projection() {
    let mut maximum = DataType::Int32;
    for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
        maximum = DataType::list(Field::new("item", maximum, false));
    }
    let array = maximum.default_arrow_array().unwrap();
    scalar_value(&Field::new("value", maximum.clone(), false), array.as_ref()).unwrap();

    let overdeep = DataType::list(Field::new("item", maximum, false));
    let unrelated: ArrayRef = Arc::new(Int32Array::from(vec![0]));
    let error = scalar_value(&Field::new("value", overdeep, false), unrelated.as_ref())
        .unwrap_err()
        .to_string();
    assert!(error.contains("hard limit"), "{error}");
}
