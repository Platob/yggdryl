use yggdryl::{DataType, Error, TimeUnit};

#[test]
fn time_selects_the_arrow_physical_width() {
    for (unit, expected) in [
        (TimeUnit::Second, DataType::Time32(TimeUnit::Second)),
        (
            TimeUnit::Millisecond,
            DataType::Time32(TimeUnit::Millisecond),
        ),
        (
            TimeUnit::Microsecond,
            DataType::Time64(TimeUnit::Microsecond),
        ),
        (TimeUnit::Nanosecond, DataType::Time64(TimeUnit::Nanosecond)),
    ] {
        assert_eq!(DataType::time(unit).unwrap(), expected);
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
    for unit in [
        TimeUnit::YearMonth,
        TimeUnit::DayTime,
        TimeUnit::MonthDayNano,
    ] {
        let error = DataType::time(unit).unwrap_err();
        assert!(
            matches!(
                error,
                Error::InvalidDataType { kind: "Time", ref reason }
                    if reason == "unit must be a temporal resolution"
            ),
            "unexpected error for {unit}: {error}"
        );
    }
}

#[test]
fn generic_time_parser_selects_and_round_trips_physical_storage() {
    for (expression, expected) in [
        ("time", DataType::Time64(TimeUnit::Microsecond)),
        ("time(s)", DataType::Time32(TimeUnit::Second)),
        ("time(3)", DataType::Time32(TimeUnit::Millisecond)),
        (
            "time(micro seconds)",
            DataType::Time64(TimeUnit::Microsecond),
        ),
        ("time(9)", DataType::Time64(TimeUnit::Nanosecond)),
    ] {
        let parsed = DataType::from_str(expression).unwrap();
        assert_eq!(parsed, expected, "{expression}");
        assert_eq!(DataType::from_str(&parsed.to_string()).unwrap(), parsed);
    }

    assert!(DataType::from_str("time(year_month)").is_err());
    assert!(DataType::from_str("time(10)").is_err());
}

#[test]
fn a_temporal_reads_the_compact_spelling_and_the_extended_one_alike() {
    use yggdryl::{Scalar, Timezone};

    let utc = DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    };
    let naive = DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::NAIVE,
    };

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
        DataType::Date32.scalar(Scalar::from("20240102")).unwrap(),
        DataType::Date32.scalar(Scalar::from("2024-01-02")).unwrap(),
    );
    assert_eq!(
        DataType::Time64(TimeUnit::Nanosecond)
            .scalar(Scalar::from("101530.5"))
            .unwrap(),
        DataType::Time64(TimeUnit::Nanosecond)
            .scalar(Scalar::from("10:15:30.5"))
            .unwrap(),
    );

    // What is not a spelling of anything is still refused, at its position.
    for held in ["2024010210153", "202401021015300", "20241302101530"] {
        assert!(naive.scalar(Scalar::from(held)).is_err(), "{held}");
    }
}
