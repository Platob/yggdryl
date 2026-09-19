//! Five temporal families: a datetime, a date, a time of day, a duration and
//! an interval, each one datatype over the leaves its widths are.

use arrow_schema::{
    DataType as ArrowDataType, IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit,
};
use yggdryl::types::{DateTimeType, DateType, DurationType, IntervalType, TimeType};
use yggdryl::{
    DataType, DataTypeId, DataTypeKind, Error, Scalar, TemporalFamily, TimeUnit, Timezone,
};

/// The refusal a temporal leaf states: the kind names the width or the
/// family, the reason what it would not carry. The constructor, the
/// hand-built leaf's `validate` and the root's all state the same one.
fn assert_refused<T: std::fmt::Debug>(result: yggdryl::Result<T>, kind: &str, reason: &str) {
    let error = result.unwrap_err();
    assert!(
        matches!(
            &error,
            Error::InvalidDataType { kind: held, reason: held_reason }
                if *held == kind && held_reason == reason
        ),
        "expected {kind}: {reason}, got {error}"
    );
}

/// The three interval layouts, which no clock counts in.
const LAYOUTS: [TimeUnit; 3] = [
    TimeUnit::YearMonth,
    TimeUnit::DayTime,
    TimeUnit::MonthDayNano,
];

#[test]
fn every_date_leaf_is_one_datatype_under_its_id() {
    // A date has no parameter: the unit is what the width means.
    for (leaf, id, unit, bits) in [
        (DateType::Date32, DataTypeId::Date32, TimeUnit::Day, 32),
        (
            DateType::Date64,
            DataTypeId::Date64,
            TimeUnit::Millisecond,
            64,
        ),
    ] {
        assert_eq!(leaf.id(), id);
        assert_eq!(leaf.as_str(), id.as_str());
        assert_eq!(leaf.unit(), unit);
        assert_eq!(leaf.bit_width(), bits);
        assert_eq!(leaf.family(), TemporalFamily::Date);
        assert_eq!(DateType::from_id(id), Some(leaf));
        assert!(leaf.validate().is_ok());
        assert_eq!(leaf.to_string(), id.as_str());
        assert!(id.is_temporal(), "{id}");

        let dtype = DataType::from(leaf);
        assert_eq!(dtype, DataType::Date(leaf));
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.kind(), DataTypeKind::Temporal);
        assert_eq!(dtype.date_type(), Some(leaf));
        assert_eq!(dtype.time_type(), None);
        assert_eq!(dtype.datetime_type(), None);
        assert_eq!(dtype.duration_type(), None);
        assert_eq!(dtype.interval_type(), None);
        assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
        assert_eq!(DateType::try_from(&dtype).unwrap(), leaf);
        assert!(dtype.validate().is_ok());
    }
    assert_eq!(DateType::ALL, [DateType::Date32, DateType::Date64]);
    assert_eq!(DateType::default(), DateType::Date32);
    assert_eq!(DateType::from_id(DataTypeId::Time32), None);

    // The two constructors are the two leaves, and `date` is the narrow one.
    assert_eq!(DataType::date32(), DataType::Date(DateType::Date32));
    assert_eq!(DataType::date64(), DataType::Date(DateType::Date64));
    assert_eq!(DataType::from_str("date").unwrap(), DataType::date32());
    assert_eq!(DataType::from_str("date64").unwrap(), DataType::date64());

    // Another family is refused by name.
    let refused = DateType::try_from(&DataType::Int64)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("expected a date datatype"), "{refused}");
    assert_eq!(DataType::Int64.date_type(), None);
}

#[test]
fn every_time_leaf_carries_its_resolution_as_a_parameter() {
    // The unit is a parameter and never a leaf: `time32(s)` and
    // `time32(ms)` are one storage with one parameter.
    assert_eq!(
        TimeType::ALL,
        [
            TimeType::Time32(TimeUnit::Millisecond),
            TimeType::Time64(TimeUnit::Microsecond),
        ]
    );
    for (leaf, id, bits, spelling) in [
        (
            TimeType::Time32(TimeUnit::Second),
            DataTypeId::Time32,
            32,
            "time32(s)",
        ),
        (
            TimeType::Time32(TimeUnit::Millisecond),
            DataTypeId::Time32,
            32,
            "time32(ms)",
        ),
        (
            TimeType::Time64(TimeUnit::Microsecond),
            DataTypeId::Time64,
            64,
            "time64(us)",
        ),
        (
            TimeType::Time64(TimeUnit::Nanosecond),
            DataTypeId::Time64,
            64,
            "time64(ns)",
        ),
    ] {
        assert_eq!(leaf.id(), id);
        assert_eq!(leaf.as_str(), id.as_str());
        assert_eq!(leaf.bit_width(), bits);
        assert_eq!(leaf.family(), TemporalFamily::Time);
        assert_eq!(TimeType::from_id(id, leaf.unit()), Some(leaf));
        assert_eq!(TimeType::for_unit(leaf.unit()).unwrap(), leaf);
        assert!(leaf.validate().is_ok());
        assert_eq!(leaf.to_string(), spelling);

        let dtype = DataType::from(leaf);
        assert_eq!(dtype, DataType::Time(leaf));
        assert_eq!(DataType::time_of(leaf).unwrap(), dtype);
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.kind(), DataTypeKind::Temporal);
        assert_eq!(dtype.time_type(), Some(leaf));
        assert_eq!(dtype.date_type(), None);
        assert_eq!(dtype.to_string(), spelling);
        assert_eq!(DataType::from_str(spelling).unwrap(), dtype);
        assert_eq!(TimeType::try_from(&dtype).unwrap(), leaf);
        assert!(dtype.validate().is_ok());
    }
    assert_eq!(
        TimeType::from_id(DataTypeId::Date32, TimeUnit::Second),
        None
    );

    // The leaf is public, so a width can carry a resolution it does not
    // hold; the constructor, `time_of`, `validate` and the root all refuse
    // it under the width's name.
    for unit in [TimeUnit::Microsecond, TimeUnit::Nanosecond, TimeUnit::Day] {
        let reason = "unit must be second or millisecond";
        assert_refused(DataType::time32(unit), "Time32", reason);
        assert_refused(DataType::time_of(TimeType::Time32(unit)), "Time32", reason);
        assert_refused(TimeType::Time32(unit).validate(), "Time32", reason);
        assert_refused(
            DataType::Time(TimeType::Time32(unit)).validate(),
            "Time32",
            reason,
        );
    }
    for unit in [TimeUnit::Second, TimeUnit::Millisecond, TimeUnit::Day] {
        let reason = "unit must be microsecond or nanosecond";
        assert_refused(DataType::time64(unit), "Time64", reason);
        assert_refused(DataType::time_of(TimeType::Time64(unit)), "Time64", reason);
        assert_refused(TimeType::Time64(unit).validate(), "Time64", reason);
    }
    for unit in LAYOUTS {
        assert_refused(
            DataType::time32(unit),
            "Time32",
            "unit must be second or millisecond",
        );
        assert_refused(
            DataType::time64(unit),
            "Time64",
            "unit must be microsecond or nanosecond",
        );
    }
}

#[test]
fn time_selects_the_arrow_physical_width() {
    for (unit, expected) in [
        (
            TimeUnit::Second,
            DataType::Time(TimeType::Time32(TimeUnit::Second)),
        ),
        (
            TimeUnit::Millisecond,
            DataType::Time(TimeType::Time32(TimeUnit::Millisecond)),
        ),
        (
            TimeUnit::Microsecond,
            DataType::Time(TimeType::Time64(TimeUnit::Microsecond)),
        ),
        (
            TimeUnit::Nanosecond,
            DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
        ),
    ] {
        assert_eq!(DataType::time(unit).unwrap(), expected);
        assert_eq!(
            DataType::Time(TimeType::for_unit(unit).unwrap()),
            expected,
            "{unit}"
        );
    }
}

#[test]
fn time_delegates_to_the_selected_explicit_constructor() {
    for unit in [TimeUnit::Second, TimeUnit::Millisecond] {
        assert_eq!(
            DataType::time(unit).unwrap(),
            DataType::time32(unit).unwrap()
        );
    }
    for unit in [TimeUnit::Microsecond, TimeUnit::Nanosecond] {
        assert_eq!(
            DataType::time(unit).unwrap(),
            DataType::time64(unit).unwrap()
        );
    }
}

#[test]
fn time_rejects_interval_layouts_before_physical_selection() {
    for unit in LAYOUTS {
        assert_refused(
            DataType::time(unit),
            "Time",
            "unit must be a temporal resolution",
        );
        assert_refused(
            TimeType::for_unit(unit),
            "Time",
            "unit must be a temporal resolution",
        );
    }
    // A day is a length and never a clock reading.
    assert_refused(
        DataType::time(TimeUnit::Day),
        "Time",
        "unit must be a temporal resolution",
    );
}

#[test]
fn generic_time_parser_selects_and_round_trips_physical_storage() {
    for (expression, expected) in [
        (
            "time",
            DataType::Time(TimeType::Time64(TimeUnit::Microsecond)),
        ),
        (
            "time(s)",
            DataType::Time(TimeType::Time32(TimeUnit::Second)),
        ),
        (
            "time(3)",
            DataType::Time(TimeType::Time32(TimeUnit::Millisecond)),
        ),
        (
            "time(micro seconds)",
            DataType::Time(TimeType::Time64(TimeUnit::Microsecond)),
        ),
        (
            "time(9)",
            DataType::Time(TimeType::Time64(TimeUnit::Nanosecond)),
        ),
    ] {
        let parsed = DataType::from_str(expression).unwrap();
        assert_eq!(parsed, expected, "{expression}");
        assert_eq!(DataType::from_str(&parsed.to_string()).unwrap(), parsed);
    }

    assert!(DataType::from_str("time(year_month)").is_err());
    assert!(DataType::from_str("time(10)").is_err());
}

#[test]
fn the_datetime_leaf_carries_a_resolution_and_a_zone() {
    // One leaf, two parameters: the resolution and the zone the counts are
    // read in. `ALL` holds it at the grammar's defaults.
    let default = DateTimeType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: Timezone::NAIVE,
    };
    assert_eq!(DateTimeType::ALL, [default]);
    assert_eq!(default.id(), DataTypeId::DateTime64);
    assert_eq!(default.as_str(), "datetime64");
    assert_eq!(default.unit(), TimeUnit::Microsecond);
    assert_eq!(default.timezone(), Timezone::NAIVE);
    assert_eq!(default.bit_width(), 64);
    assert_eq!(default.family(), TemporalFamily::DateTime);
    assert_eq!(
        DateTimeType::from_id(
            DataTypeId::DateTime64,
            TimeUnit::Microsecond,
            Timezone::NAIVE
        ),
        Some(default)
    );
    assert_eq!(
        DateTimeType::from_id(DataTypeId::Date32, TimeUnit::Microsecond, Timezone::NAIVE),
        None
    );
    assert_eq!(default.to_string(), "datetime64(us)");
    assert_eq!(
        DataType::from_str("timestamp").unwrap(),
        DataType::DateTime(default)
    );

    // Every clock resolution is valid in every zone, and the constructor
    // answers the literal leaf.
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        for timezone in [Timezone::NAIVE, Timezone::UTC] {
            let leaf = DateTimeType::DateTime64 { unit, timezone };
            let dtype = DataType::datetime64(unit, timezone).unwrap();
            assert_eq!(dtype, DataType::DateTime(leaf));
            assert_eq!(DataType::from(leaf), dtype);
            assert_eq!(dtype.id(), DataTypeId::DateTime64);
            assert_eq!(dtype.kind(), DataTypeKind::Temporal);
            assert_eq!(dtype.datetime_type(), Some(leaf));
            assert_eq!(dtype.date_type(), None);
            assert_eq!(DateTimeType::try_from(&dtype).unwrap(), leaf);
            assert_eq!(
                default.with_unit(unit).unwrap().with_timezone(timezone),
                leaf
            );
            assert!(leaf.validate().is_ok());
            assert!(dtype.validate().is_ok());
            assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
        }
    }
    // The zone is quoted beside the unit; a wall clock states none.
    assert_eq!(
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)
            .unwrap()
            .to_string(),
        "datetime64(us,\"UTC\")"
    );

    // A day and the interval layouts are no resolution of a clock, and the
    // one rule is stated under the leaf's name wherever it is checked.
    for unit in [
        TimeUnit::Day,
        TimeUnit::YearMonth,
        TimeUnit::DayTime,
        TimeUnit::MonthDayNano,
    ] {
        let reason = "unit must be a temporal resolution";
        assert_refused(
            DataType::datetime64(unit, Timezone::NAIVE),
            "datetime64",
            reason,
        );
        assert_refused(default.with_unit(unit), "datetime64", reason);
        let leaf = DateTimeType::DateTime64 {
            unit,
            timezone: Timezone::UTC,
        };
        assert_refused(leaf.validate(), "datetime64", reason);
        assert_refused(DataType::DateTime(leaf).validate(), "datetime64", reason);
    }
}

#[test]
fn every_duration_leaf_carries_a_fixed_length_unit() {
    // Both widths carry every fixed-length unit, a day included: a day is a
    // length where a clock has none.
    assert_eq!(
        DurationType::ALL,
        [
            DurationType::Duration32(TimeUnit::Millisecond),
            DurationType::Duration64(TimeUnit::Microsecond),
        ]
    );
    let fixed = [
        TimeUnit::Day,
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ];
    for unit in fixed {
        for (leaf, id, bits, constructed) in [
            (
                DurationType::Duration32(unit),
                DataTypeId::Duration32,
                32,
                DataType::duration32(unit).unwrap(),
            ),
            (
                DurationType::Duration64(unit),
                DataTypeId::Duration64,
                64,
                DataType::duration64(unit).unwrap(),
            ),
        ] {
            assert_eq!(leaf.id(), id);
            assert_eq!(leaf.as_str(), id.as_str());
            assert_eq!(leaf.unit(), unit);
            assert_eq!(leaf.bit_width(), bits);
            assert_eq!(leaf.family(), TemporalFamily::Duration);
            assert_eq!(DurationType::from_id(id, unit), Some(leaf));
            assert!(leaf.validate().is_ok());
            assert_eq!(leaf.to_string(), format!("{}({unit})", id.as_str()));

            assert_eq!(constructed, DataType::Duration(leaf));
            assert_eq!(DataType::duration_of(leaf).unwrap(), constructed);
            assert_eq!(DataType::from(leaf), constructed);
            assert_eq!(constructed.id(), id);
            assert_eq!(constructed.kind(), DataTypeKind::Temporal);
            assert_eq!(constructed.duration_type(), Some(leaf));
            assert_eq!(constructed.time_type(), None);
            assert_eq!(DurationType::try_from(&constructed).unwrap(), leaf);
            assert!(constructed.validate().is_ok());
            // The grammar reads a clock resolution after `duration32` and
            // `duration64`; a day is a length the constructors take and the
            // spelling renders, but not one the grammar reads back.
            if unit == TimeUnit::Day {
                assert!(DataType::from_str(&constructed.to_string()).is_err());
            } else {
                assert_eq!(
                    DataType::from_str(&constructed.to_string()).unwrap(),
                    constructed
                );
            }
        }
    }
    assert_eq!(
        DurationType::from_id(DataTypeId::Interval, TimeUnit::Second),
        None
    );
    assert_eq!(
        DataType::duration64(TimeUnit::Nanosecond)
            .unwrap()
            .to_string(),
        "duration64(ns)"
    );
    // Arrow's debug spelling `Duration(unit)` reads as the 64-bit width Arrow
    // stores every duration in; a bare lowercase `duration` names no width
    // and stays unknown rather than becoming a width-ambiguous alias.
    assert_eq!(
        DataType::from_str("Duration(us)").unwrap(),
        DataType::Duration(DurationType::Duration64(TimeUnit::Microsecond))
    );
    assert!(DataType::from_str("duration").is_err());

    // An interval layout is no length at all, refused under the width's
    // name by the constructor, `duration_of`, `validate` and the root.
    for unit in LAYOUTS {
        let reason = "unit must be day, second, millisecond, microsecond, or nanosecond";
        assert_refused(DataType::duration32(unit), "Duration32", reason);
        assert_refused(DataType::duration64(unit), "Duration64", reason);
        assert_refused(
            DataType::duration_of(DurationType::Duration32(unit)),
            "Duration32",
            reason,
        );
        assert_refused(
            DurationType::Duration64(unit).validate(),
            "Duration64",
            reason,
        );
        assert_refused(
            DataType::Duration(DurationType::Duration32(unit)).validate(),
            "Duration32",
            reason,
        );
    }
}

#[test]
fn the_interval_leaf_carries_its_layout() {
    // One leaf; the three layouts stay its `TimeUnit` parameter.
    assert_eq!(
        IntervalType::ALL,
        [IntervalType::Interval(TimeUnit::MonthDayNano)]
    );
    for (unit, spelling) in [
        (TimeUnit::YearMonth, "interval(year_month)"),
        (TimeUnit::DayTime, "interval(day_time)"),
        (TimeUnit::MonthDayNano, "interval(month_day_nano)"),
    ] {
        let leaf = IntervalType::Interval(unit);
        assert_eq!(leaf.id(), DataTypeId::Interval);
        assert_eq!(leaf.as_str(), "interval");
        assert_eq!(leaf.unit(), unit);
        assert_eq!(leaf.family(), TemporalFamily::Interval);
        assert_eq!(
            IntervalType::from_id(DataTypeId::Interval, unit),
            Some(leaf)
        );
        assert!(leaf.validate().is_ok());
        assert_eq!(leaf.to_string(), spelling);

        // The constructor validates the layout where the bare variant did
        // not, and answers the literal leaf.
        let dtype = DataType::interval(unit).unwrap();
        assert_eq!(dtype, DataType::Interval(leaf));
        assert_eq!(DataType::from(leaf), dtype);
        assert_eq!(dtype.id(), DataTypeId::Interval);
        assert_eq!(dtype.kind(), DataTypeKind::Temporal);
        assert_eq!(dtype.interval_type(), Some(leaf));
        assert_eq!(dtype.duration_type(), None);
        assert_eq!(IntervalType::try_from(&dtype).unwrap(), leaf);
        assert!(dtype.validate().is_ok());
        assert_eq!(dtype.to_string(), spelling);
        assert_eq!(DataType::from_str(spelling).unwrap(), dtype);
    }
    assert_eq!(
        IntervalType::from_id(DataTypeId::Duration64, TimeUnit::DayTime),
        None
    );
    assert_eq!(
        DataType::from_str("interval").unwrap(),
        DataType::interval(TimeUnit::MonthDayNano).unwrap()
    );

    // A resolution is no layout, and the leaf is public, so a hand-built
    // one is caught where a constructor would have refused it.
    for unit in [
        TimeUnit::Day,
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        let reason = "unit must be an interval layout";
        assert_refused(DataType::interval(unit), "Interval", reason);
        assert_refused(IntervalType::Interval(unit).validate(), "Interval", reason);
        assert_refused(
            DataType::Interval(IntervalType::Interval(unit)).validate(),
            "Interval",
            reason,
        );
    }
}

#[test]
fn every_temporal_leaf_projects_to_its_arrow_storage_and_imports_back() {
    // Each family lays out its own storage, and the storage imports as the
    // leaf that declared it.
    let cases = [
        (DataType::date32(), ArrowDataType::Date32),
        (DataType::date64(), ArrowDataType::Date64),
        (
            DataType::time32(TimeUnit::Second).unwrap(),
            ArrowDataType::Time32(ArrowTimeUnit::Second),
        ),
        (
            DataType::time32(TimeUnit::Millisecond).unwrap(),
            ArrowDataType::Time32(ArrowTimeUnit::Millisecond),
        ),
        (
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            ArrowDataType::Time64(ArrowTimeUnit::Microsecond),
        ),
        (
            DataType::time64(TimeUnit::Nanosecond).unwrap(),
            ArrowDataType::Time64(ArrowTimeUnit::Nanosecond),
        ),
        (
            DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, None),
        ),
        (
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, Some("UTC".into())),
        ),
        (
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
            ArrowDataType::Duration(ArrowTimeUnit::Millisecond),
        ),
        (
            DataType::interval(TimeUnit::YearMonth).unwrap(),
            ArrowDataType::Interval(ArrowIntervalUnit::YearMonth),
        ),
        (
            DataType::interval(TimeUnit::DayTime).unwrap(),
            ArrowDataType::Interval(ArrowIntervalUnit::DayTime),
        ),
        (
            DataType::interval(TimeUnit::MonthDayNano).unwrap(),
            ArrowDataType::Interval(ArrowIntervalUnit::MonthDayNano),
        ),
    ];
    for (dtype, storage) in cases {
        assert_eq!(dtype.to_arrow_datatype().unwrap(), storage, "{dtype}");
        assert_eq!(
            dtype.clone().into_arrow_datatype().unwrap(),
            storage,
            "{dtype}"
        );
        assert_eq!(
            DataType::from_arrow_datatype(&storage).unwrap(),
            dtype,
            "{dtype}"
        );
    }

    // Arrow has one duration width, so the 32-bit leaf widens on the way
    // out and comes back as the 64-bit one.
    assert_eq!(
        DataType::duration32(TimeUnit::Second)
            .unwrap()
            .to_arrow_datatype()
            .unwrap(),
        ArrowDataType::Duration(ArrowTimeUnit::Second)
    );
    assert_eq!(
        DataType::from_arrow_datatype(&ArrowDataType::Duration(ArrowTimeUnit::Second)).unwrap(),
        DataType::duration64(TimeUnit::Second).unwrap()
    );

    // A hand-built leaf its width does not carry stops at the boundary
    // rather than widening silently.
    for dtype in [
        DataType::Time(TimeType::Time32(TimeUnit::Nanosecond)),
        DataType::Time(TimeType::Time64(TimeUnit::Second)),
        DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Day,
            timezone: Timezone::NAIVE,
        }),
        DataType::Duration(DurationType::Duration32(TimeUnit::YearMonth)),
        DataType::Interval(IntervalType::Interval(TimeUnit::Second)),
    ] {
        assert!(dtype.to_arrow_datatype().is_err(), "{dtype}");
        assert!(dtype.into_arrow_datatype().is_err());
    }
}

#[test]
fn a_temporal_reads_the_compact_spelling_and_the_extended_one_alike() {
    let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap();
    let naive = DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();

    // One instant, five spellings: extended, compact, the FIX form that mixes
    // them, and either with a fraction. A wire that writes every byte it can
    // spare writes the compact one, and a log line quoting it is the same
    // value.
    let stated = naive
        .scalar(Scalar::from("2024-01-02T10:15:30"))
        .expect("the extended spelling");
    for held in ["20240102101530", "20240102-10:15:30", "2024-01-02 10:15:30"] {
        assert_eq!(naive.scalar(Scalar::from(held)).unwrap(), stated, "{held}");
    }

    // The fraction reads at its own width whichever spelling carried it, and
    // widens the unit exactly as it always did.
    let fractional = naive
        .scalar(Scalar::from("2024-01-02T10:15:30.123456789"))
        .expect("a nanosecond fraction");
    assert_eq!(
        naive
            .scalar(Scalar::from("20240102101530.123456789"))
            .unwrap(),
        fractional,
    );
    assert_ne!(fractional, stated);

    // A zone still closes a zoned reading, compact or not.
    let zoned = utc
        .scalar(Scalar::from("2024-01-02T10:15:30Z"))
        .expect("the extended spelling");
    for held in [
        "20240102101530Z",
        "20240102-10:15:30Z",
        "20240102101530+00:00",
    ] {
        assert_eq!(utc.scalar(Scalar::from(held)).unwrap(), zoned, "{held}");
    }

    // A date and a time of day read compactly on their own too.
    assert_eq!(
        DataType::date32().scalar(Scalar::from("20240102")).unwrap(),
        DataType::date32()
            .scalar(Scalar::from("2024-01-02"))
            .unwrap(),
    );
    let nanos = DataType::time64(TimeUnit::Nanosecond).unwrap();
    assert_eq!(
        nanos.scalar(Scalar::from("101530.5")).unwrap(),
        nanos.scalar(Scalar::from("10:15:30.5")).unwrap(),
    );

    // A date states no clock, so a datetime column reads one as that day's
    // midnight - which is how a settlement date sits in the column a transact
    // time sits in, rather than in a second one to cast through.
    let midnight = naive
        .scalar(Scalar::from("2024-01-02T00:00:00"))
        .expect("the extended midnight");
    for held in ["2024-01-02", "20240102"] {
        assert_eq!(
            naive.scalar(Scalar::from(held)).unwrap(),
            midnight,
            "{held}"
        );
    }

    // A zoned column still wants the zone stated, and reads the same midnight
    // once it is; a date that states none is a naive reading and stays one.
    let zoned_midnight = utc
        .scalar(Scalar::from("2024-01-02T00:00:00Z"))
        .expect("the extended midnight");
    for held in ["2024-01-02Z", "20240102Z", "20240102+00:00"] {
        assert_eq!(
            utc.scalar(Scalar::from(held)).unwrap(),
            zoned_midnight,
            "{held}"
        );
    }
    assert!(utc.scalar(Scalar::from("20240102")).is_err());
    assert!(naive.scalar(Scalar::from("20240102Z")).is_err());

    // What is not a spelling of anything is still refused, at its position.
    for held in ["2024010210153", "202401021015300", "20241302101530"] {
        assert!(naive.scalar(Scalar::from(held)).is_err(), "{held}");
    }
}

#[test]
fn either_decimal_sign_reads_the_same_fraction() {
    let naive = DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE).unwrap();

    // ISO 8601 divides a fraction from its integer part with either decimal
    // sign and names the comma the preferred one, so a log4j row and a
    // European locale's clock read the instant their dotted twin reads - in
    // every spelling of the datetime that carries them.
    let stated = naive
        .scalar(Scalar::from("2026-08-14T00:05:01.148"))
        .expect("the full stop");
    for held in [
        "2026-08-14T00:05:01,148",
        "2026-08-14 00:05:01,148",
        "20260814-00:05:01,148",
    ] {
        assert_eq!(naive.scalar(Scalar::from(held)).unwrap(), stated, "{held}");
    }

    // A comma closes a zoned reading too, where the zone follows the fraction.
    let utc = DataType::datetime64(TimeUnit::Millisecond, Timezone::UTC).unwrap();
    assert_eq!(
        utc.scalar(Scalar::from("2026-08-14T00:05:01,148Z"))
            .unwrap(),
        utc.scalar(Scalar::from("2026-08-14T00:05:01.148Z"))
            .unwrap(),
    );

    // A comma still needs its digits, and a grouped fraction reads at the
    // width the grouping spells.
    assert!(naive.scalar(Scalar::from("2026-08-14T00:05:01,")).is_err());
    let micros = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap();
    assert_eq!(
        micros
            .scalar(Scalar::from("2026-08-14 00:05:01,147_250"))
            .unwrap(),
        micros
            .scalar(Scalar::from("2026-08-14 00:05:01.147250"))
            .unwrap(),
    );
}
