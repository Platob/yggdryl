//! `rust/src/datetime.rs`: five temporal families: a datetime, a date, a
//! time of day, a duration and an interval, each one datatype over the
//! leaves its widths are.

mod temporal {

    use yggdryl::DateTimeType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Error, TimeUnit, Timezone};

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
        assert_eq!(default.family(), "datetime");
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
}
