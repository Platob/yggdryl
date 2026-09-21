//! `rust/src/interval.rs`: five temporal families: a datetime, a date, a
//! time of day, a duration and an interval, each one datatype over the
//! leaves its widths are.

mod temporal {

    use yggdryl::IntervalType;
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
            assert_eq!(leaf.family(), "interval");
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
}
