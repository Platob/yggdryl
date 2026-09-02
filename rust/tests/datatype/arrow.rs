use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
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
        core.get_field_by_name("name").unwrap().dtype(),
        &DataType::Utf8
    );
}

#[test]
fn every_temporal_and_interval_unit_round_trips_through_all_core_formats() {
    let values = [
        DataType::Timestamp(TimeUnit::Second, None),
        DataType::Timestamp(TimeUnit::Millisecond, Some(Timezone::UTC)),
        DataType::Timestamp(
            TimeUnit::Microsecond,
            Some(Timezone::from_str("Europe/Paris").unwrap()),
        ),
        DataType::Timestamp(TimeUnit::Nanosecond, None),
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
    let item = || Field::new("item", DataType::Utf8, true);
    let entries = || {
        Field::new(
            "entries",
            DataType::from_fields([
                Field::new("key", DataType::Utf8, false),
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
        DataType::Timestamp(
            TimeUnit::Nanosecond,
            Some(Timezone::from_str("Europe/Paris").unwrap()),
        ),
        DataType::Date32,
        DataType::Date64,
        DataType::Time32(TimeUnit::Millisecond),
        DataType::Time64(TimeUnit::Microsecond),
        DataType::Duration64(TimeUnit::Nanosecond),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::Binary,
        DataType::fixed_size_binary(16).unwrap(),
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::list(item()),
        DataType::list_view(item()),
        DataType::fixed_size_list(item(), 4).unwrap(),
        DataType::large_list(item()),
        DataType::large_list_view(item()),
        DataType::from_fields([Field::new("value", DataType::Int32, false)]).unwrap(),
        DataType::union(
            [
                (0, Field::new("number", DataType::Int64, false)),
                (7, Field::new("text", DataType::Utf8, true)),
            ],
            UnionMode::Dense,
        )
        .unwrap(),
        DataType::dictionary(DataType::UInt16, DataType::Utf8).unwrap(),
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, -2).unwrap(),
        DataType::decimal128(38, 18).unwrap(),
        DataType::decimal256(76, 20).unwrap(),
        DataType::map(entries(), true).unwrap(),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
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

#[test]
fn invalid_arrow_parameters_and_nested_shapes_fail_before_projection() {
    assert!(DataType::Time32(TimeUnit::Nanosecond).validate().is_err());
    assert!(DataType::Time64(TimeUnit::Second).validate().is_err());
    for invalid in [
        DataType::Timestamp(TimeUnit::YearMonth, None),
        DataType::Duration32(TimeUnit::DayTime),
        DataType::Duration64(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::Second),
    ] {
        assert!(invalid.validate().is_err());
        assert!(invalid.clone().into_arrow().is_err());
        assert!(invalid.into_json().is_err());
    }
    assert!(DataType::fixed_size_binary(-1).is_err());
    assert!(DataType::fixed_size_list(Field::new("item", DataType::Utf8, true), -1).is_err());
    assert!(DataType::decimal128(0, 0).is_err());
    assert!(DataType::decimal128(5, 6).is_err());
    assert!(DataType::dictionary(DataType::Float64, DataType::Utf8).is_err());
    assert!(
        DataType::map(
            Field::new(
                "entries",
                DataType::from_fields([
                    Field::new("key", DataType::Utf8, true),
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
            Field::new("values", DataType::Utf8, true),
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

    let invalid_binary = DataType::FixedSizeBinary(-1);
    for error in [
        DataType::fixed_size_binary(-1).unwrap_err(),
        invalid_binary.validate().unwrap_err(),
        invalid_binary.clone().into_arrow().unwrap_err(),
        invalid_binary.into_arrow_ffi().unwrap_err(),
    ] {
        assert_invalid(error, "FixedSizeBinary", "width must be non-negative: -1");
    }

    let item = Field::new("item", DataType::Utf8, true);
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
