//! `rust/src/time.rs`: five temporal families: a datetime, a date, a time
//! of day, a duration and an interval, each one datatype over the leaves
//! its widths are.

mod temporal {

    use yggdryl::TimeType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Error, TimeUnit};

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
            assert_eq!(leaf.family(), "time");
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
}
