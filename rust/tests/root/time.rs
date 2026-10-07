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
            assert_eq!(leaf.id().temporal_family(), Some("time"));
            assert_eq!(TimeType::from_id(id, leaf.unit()), Some(leaf));
            assert_eq!(TimeType::for_unit(leaf.unit()).unwrap(), leaf);
            assert!(leaf.validate().is_ok());
            assert_eq!(leaf.to_string(), spelling);

            let dtype = DataType::from(leaf);
            assert_eq!(dtype, DataType::from(leaf));
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
            assert_refused(DataType::Time32(unit).validate(), "Time32", reason);
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
            assert_eq!(
                DataType::from(TimeType::for_unit(unit).unwrap()),
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
}

/// `Time32::from_text` and `Time64::from_text`, the ISO doors of the two
/// clocks, and the short spelling every clock renders through.
mod from_text {
    use yggdryl::{Error, Scalar, Time32, Time64, TimeUnit, Timezone};

    /// 09:30:00 in microseconds.
    const HALF_PAST_NINE: i64 = 34_200_000_000;

    #[test]
    fn a_zoned_clock_is_refused_naming_the_type_that_reads_one() {
        // The refusal first: an offset makes a clock an instant, and the
        // refusal says where one reads rather than reporting trailing text.
        for text in ["09:30:00Z", "09:30:00z", "09:30:00+05:30", "09:30:00-08:00"] {
            for error in [
                Time64::from_text(text).unwrap_err(),
                Time32::from_text(text).unwrap_err(),
            ] {
                assert!(
                    matches!(&error, Error::InvalidRecord { reason, .. } if reason.contains("DateTime64")),
                    "{text:?}: {error}"
                );
            }
        }
        // A clock that stops at its minutes is FIX's spelling, not ISO's,
        // and the refusal names the byte the seconds were owed at.
        for (text, position) in [("09:30", 5), ("0930", 4), ("", 0), ("soon", 0)] {
            for error in [
                Time64::from_text(text).unwrap_err(),
                Time32::from_text(text).unwrap_err(),
            ] {
                assert!(
                    matches!(&error, Error::Parse { target: "time", position: held, .. } if *held == position),
                    "{text:?}: {error}"
                );
            }
        }
        // Minutes and seconds stay under sixty; a leap second is no clock.
        for text in ["09:60:00", "09:30:60", "093060"] {
            assert!(Time64::from_text(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn time64_reads_the_digits_unit_widened_to_microseconds() {
        for (text, count, unit) in [
            ("09:30:00", HALF_PAST_NINE, TimeUnit::Microsecond),
            ("093000", HALF_PAST_NINE, TimeUnit::Microsecond),
            // `.5` widths: one digit is milliseconds, held at the floor.
            (
                "09:30:00.5",
                HALF_PAST_NINE + 500_000,
                TimeUnit::Microsecond,
            ),
            (
                "09:30:00,500",
                HALF_PAST_NINE + 500_000,
                TimeUnit::Microsecond,
            ),
            (
                "09:30:00.500000",
                HALF_PAST_NINE + 500_000,
                TimeUnit::Microsecond,
            ),
            ("09:30:00.000001", HALF_PAST_NINE + 1, TimeUnit::Microsecond),
            (
                "09:30:00.500000000",
                HALF_PAST_NINE * 1_000 + 500_000_000,
                TimeUnit::Nanosecond,
            ),
            (
                "093000.000000001",
                HALF_PAST_NINE * 1_000 + 1,
                TimeUnit::Nanosecond,
            ),
            // An hour past the day folds into it.
            ("25:00:00", 3_600_000_000, TimeUnit::Microsecond),
            ("24:00:00", 0, TimeUnit::Microsecond),
        ] {
            let read = Time64::from_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
            assert!(read.timezone().is_naive());
        }
    }

    #[test]
    fn time32_reads_the_digits_unit_and_narrows_a_fraction_it_holds() {
        for (text, count, unit) in [
            ("09:30:00", 34_200, TimeUnit::Second),
            ("093000", 34_200, TimeUnit::Second),
            ("09:30:00.5", 34_200_500, TimeUnit::Millisecond),
            ("09:30:00.500", 34_200_500, TimeUnit::Millisecond),
            // A finer spelling of a fraction the width holds narrows to it.
            ("09:30:00.000000", 34_200_000, TimeUnit::Millisecond),
            ("09:30:00.500000000", 34_200_500, TimeUnit::Millisecond),
            ("25:00:00", 3_600, TimeUnit::Second),
        ] {
            let read = Time32::from_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
        }
        // A fraction the width cannot hold is refused naming the width that can.
        for text in ["09:30:00.000001", "09:30:00.500000001", "093000.0001"] {
            let error = Time32::from_text(text).unwrap_err();
            assert!(
                matches!(&error, Error::InvalidRecord { reason, .. } if reason.contains("Time64")),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn a_clock_prints_no_fraction_where_it_is_zero_and_the_shortest_exact_one_otherwise() {
        // `Display` is the classic spelling, so a typed cell prints as a
        // reader would write it: at every unit, a zero fraction spells
        // nothing and any other the shortest of three, six or nine digits.
        for (count, unit, spelled) in [
            (34_200, TimeUnit::Second, "09:30:00"),
            (34_200_000, TimeUnit::Millisecond, "09:30:00"),
            (34_200_500, TimeUnit::Millisecond, "09:30:00.500"),
            (34_200_001, TimeUnit::Millisecond, "09:30:00.001"),
        ] {
            let value = Time32::new(count, unit, Timezone::NAIVE).unwrap();
            assert_eq!(value.to_string(), spelled, "{count} {unit}");
            // The round trip: the spelling read at the column's unit is the count.
            let read = Time32::from_text(spelled).unwrap();
            assert_eq!(
                Scalar::Time32(read).temporal_count_at(unit),
                Some(i64::from(count)),
                "{spelled}"
            );
        }
        for (count, unit, spelled) in [
            (HALF_PAST_NINE, TimeUnit::Microsecond, "09:30:00"),
            (
                HALF_PAST_NINE + 500_000,
                TimeUnit::Microsecond,
                "09:30:00.500",
            ),
            (HALF_PAST_NINE + 1, TimeUnit::Microsecond, "09:30:00.000001"),
            (HALF_PAST_NINE * 1_000, TimeUnit::Nanosecond, "09:30:00"),
            (
                HALF_PAST_NINE * 1_000 + 500_000_000,
                TimeUnit::Nanosecond,
                "09:30:00.500",
            ),
            (
                HALF_PAST_NINE * 1_000 + 1_000,
                TimeUnit::Nanosecond,
                "09:30:00.000001",
            ),
            (
                HALF_PAST_NINE * 1_000 + 1,
                TimeUnit::Nanosecond,
                "09:30:00.000000001",
            ),
            (0, TimeUnit::Nanosecond, "00:00:00"),
        ] {
            let value = Time64::new(count, unit, Timezone::NAIVE).unwrap();
            assert_eq!(value.to_string(), spelled, "{count} {unit}");
            let read = Time64::from_text(spelled).unwrap();
            assert_eq!(
                Scalar::Time64(read).temporal_count_at(unit),
                Some(count),
                "{spelled}"
            );
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The FIX doors of the two clocks, which the FIX codec alone reaches.

    use yggdryl::internals::time::{time32_from_fix_text, time64_from_fix_text};
    use yggdryl::{Error, TimeUnit};

    #[test]
    fn the_fix_door_refuses_what_fix_does_not_write() {
        // A run of fraction digits that is not three, six or nine says
        // nothing about where the seconds end; a leap second is no clock;
        // the empty text is nothing; a zone names `DateTime64` as the ISO
        // door does.
        for (text, position) in [("0930001", 6), ("09300012", 6), ("093000123456789012", 6)] {
            let error = time64_from_fix_text(text).unwrap_err();
            assert!(
                matches!(&error, Error::Parse { target: "time", position: held, .. } if *held == position),
                "{text:?}: {error}"
            );
            assert!(time32_from_fix_text(text).is_err(), "{text:?}");
        }
        for text in ["09:30:60", "093060", "", "soon", "09:30:00 "] {
            assert!(time64_from_fix_text(text).is_err(), "{text:?}");
            assert!(time32_from_fix_text(text).is_err(), "{text:?}");
        }
        for text in ["09:30Z", "0930+05:30", "093000123+05:30"] {
            let error = time64_from_fix_text(text).unwrap_err();
            assert!(
                matches!(&error, Error::InvalidRecord { reason, .. } if reason.contains("DateTime64")),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn the_fix_door_reads_a_minute_clock_and_the_unseparated_fraction_run() {
        for (text, count, unit) in [
            // Everything the ISO door reads.
            ("09:30:00", 34_200_000_000, TimeUnit::Microsecond),
            ("093000", 34_200_000_000, TimeUnit::Microsecond),
            ("09:30:00.5", 34_200_500_000, TimeUnit::Microsecond),
            // A clock that stops at its minutes.
            ("09:30", 34_200_000_000, TimeUnit::Microsecond),
            ("0930", 34_200_000_000, TimeUnit::Microsecond),
            // The digit run with its fraction unseparated.
            ("093000123", 34_200_123_000, TimeUnit::Microsecond),
            ("093000123456", 34_200_123_456, TimeUnit::Microsecond),
            ("093000123456789", 34_200_123_456_789, TimeUnit::Nanosecond),
            // The hours fold as the ISO door's do.
            ("2500", 3_600_000_000, TimeUnit::Microsecond),
        ] {
            let read =
                time64_from_fix_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
        }
        for (text, count, unit) in [
            ("09:30", 34_200, TimeUnit::Second),
            ("0930", 34_200, TimeUnit::Second),
            ("093000123", 34_200_123, TimeUnit::Millisecond),
            ("093000123000", 34_200_123, TimeUnit::Millisecond),
        ] {
            let read =
                time32_from_fix_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
        }
        let error = time32_from_fix_text("093000123456").unwrap_err();
        assert!(
            matches!(&error, Error::InvalidRecord { reason, .. } if reason.contains("Time64")),
            "{error}"
        );
    }
}
