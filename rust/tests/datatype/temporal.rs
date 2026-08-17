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
