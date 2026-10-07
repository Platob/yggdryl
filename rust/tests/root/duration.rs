//! `rust/src/duration.rs`: five temporal families: a datetime, a date, a
//! time of day, a duration and an interval, each one datatype over the
//! leaves its widths are.

mod temporal {

    use yggdryl::DurationType;
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
                assert_eq!(leaf.id().temporal_family(), Some("duration"));
                assert_eq!(DurationType::from_id(id, unit), Some(leaf));
                assert!(leaf.validate().is_ok());
                assert_eq!(leaf.to_string(), format!("{}({unit})", id.as_str()));

                assert_eq!(constructed, DataType::from(leaf));
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
            DataType::Duration64(TimeUnit::Microsecond)
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
            assert_refused(DataType::Duration32(unit).validate(), "Duration32", reason);
        }
    }
}

#[cfg(all(feature = "internals", feature = "http"))]
mod internal {
    //! The one reader a setting's elapsed length goes through, which no
    //! caller names; every such setting is the HTTP client's, so the reader
    //! exists under that feature.

    use std::time::Duration;

    use yggdryl::internals::duration::{DURATION_SPELLINGS, duration_from_text};

    #[test]
    fn a_setting_reads_seconds_with_a_fraction_and_one_fixed_length_unit() {
        for (text, expected) in [
            ("30", Duration::from_secs(30)),
            (" 30 ", Duration::from_secs(30)),
            ("+30", Duration::from_secs(30)),
            ("0", Duration::ZERO),
            ("-0", Duration::ZERO),
            ("2.5", Duration::from_millis(2_500)),
            (".5", Duration::from_millis(500)),
            ("5.", Duration::from_secs(5)),
            ("1e3", Duration::from_secs(1_000)),
            ("1.5e-3", Duration::from_micros(1_500)),
            ("30s", Duration::from_secs(30)),
            ("2 S", Duration::from_secs(2)),
            ("30 seconds", Duration::from_secs(30)),
            ("250ms", Duration::from_millis(250)),
            ("0.5ms", Duration::from_micros(500)),
            ("1500 millis", Duration::from_millis(1_500)),
            ("7us", Duration::from_micros(7)),
            ("7\u{b5}s", Duration::from_micros(7)),
            ("250ns", Duration::from_nanos(250)),
            ("1d", Duration::from_secs(86_400)),
            ("1.5 days", Duration::from_secs(129_600)),
            // A whole count is exact at every magnitude.
            (
                "9007199254740993",
                Duration::from_secs(9_007_199_254_740_993),
            ),
            ("18446744073709551615", Duration::from_secs(u64::MAX)),
            ("18446744073709551615ns", Duration::from_nanos(u64::MAX)),
        ] {
            assert_eq!(duration_from_text(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn a_length_no_setting_spells_is_refused_never_a_panic() {
        for text in [
            "",
            " ",
            "s",
            "ms",
            "-1",
            "-0.5",
            "-1s",
            "1m",
            "1h",
            "1 min",
            "1 hour",
            "1 month",
            "1e30",
            "1e30ms",
            "1e400",
            "inf",
            "nan",
            "1.2.3",
            "1e",
            "1 s s",
            "soon",
            "1,5",
            "213503982334602d",
            "1 year",
            "PT30S",
            "00:00:30",
        ] {
            assert_eq!(duration_from_text(text), None, "{text:?}");
        }
        assert_eq!(
            DURATION_SPELLINGS,
            "seconds, with an optional fraction and an optional s, ms, us, ns or d unit"
        );
    }
}

/// `Duration32::from_text` and `Duration64::from_text`, the ISO doors of
/// the duration family, and the short spelling a duration renders through.
mod from_text {
    use yggdryl::{Duration32, Duration64, Error, Scalar, TimeUnit, Timezone};

    #[test]
    fn a_settings_grammar_is_no_cell_and_a_count_past_the_width_is_refused() {
        // The refusal first: `30s` and `1.5` are what a setting spells, and
        // a cell reads neither - two grammars, each refusing the other's
        // spelling where it is read.
        for text in [
            "30s", "1.5", "250ms", "1d", "1e3", "", "later", "PT1.5M", "01:30",
        ] {
            let error = Duration64::from_text(text).unwrap_err();
            assert!(
                matches!(
                    &error,
                    Error::Parse {
                        target: "duration",
                        ..
                    }
                ),
                "{text:?}: {error}"
            );
            assert!(Duration32::from_text(text).is_err(), "{text:?}");
        }
        // The narrow width refuses a count it cannot hold by name.
        for text in ["PT3000000000S", "-PT2147483649S", "PT2147483.648S"] {
            let error = Duration32::from_text(text).unwrap_err();
            assert!(
                matches!(&error, Error::InvalidRecord { reason, .. } if reason.contains("duration32")),
                "{text:?}: {error}"
            );
            assert!(Duration64::from_text(text).is_ok(), "{text:?}");
        }
    }

    #[test]
    fn both_grammars_read_the_same_count_at_the_fractions_unit() {
        for (text, count, unit) in [
            ("PT90S", 90, TimeUnit::Second),
            ("00:01:30", 90, TimeUnit::Second),
            ("-P1DT2H3M4.5S", -93_784_500, TimeUnit::Millisecond),
            ("-26:03:04.500", -93_784_500, TimeUnit::Millisecond),
            ("25:30:00", 91_800, TimeUnit::Second),
            ("PT1,5S", 1_500, TimeUnit::Millisecond),
            ("PT0.000001S", 1, TimeUnit::Microsecond),
            ("-PT0.000000001S", -1, TimeUnit::Nanosecond),
            ("PT2147483647S", i64::from(i32::MAX), TimeUnit::Second),
        ] {
            let read =
                Duration64::from_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
            assert!(read.timezone().is_naive());
            let narrow =
                Duration32::from_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(
                (i64::from(narrow.count()), narrow.unit()),
                (count, unit),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_duration_prints_no_fraction_where_it_is_zero_and_the_shortest_exact_one_otherwise() {
        for (count, unit, spelled) in [
            (90, TimeUnit::Second, "PT90S"),
            (90_000, TimeUnit::Millisecond, "PT90S"),
            (90_000_000, TimeUnit::Microsecond, "PT90S"),
            (90_000_000_000, TimeUnit::Nanosecond, "PT90S"),
            (1_500, TimeUnit::Millisecond, "PT1.500S"),
            (1_500_000, TimeUnit::Microsecond, "PT1.500S"),
            (1_500_000_000, TimeUnit::Nanosecond, "PT1.500S"),
            (-1_500, TimeUnit::Millisecond, "-PT1.500S"),
            (1, TimeUnit::Microsecond, "PT0.000001S"),
            (1_000, TimeUnit::Nanosecond, "PT0.000001S"),
            (1, TimeUnit::Nanosecond, "PT0.000000001S"),
            (0, TimeUnit::Nanosecond, "PT0S"),
        ] {
            let value = Duration64::new(count, unit, Timezone::NAIVE).unwrap();
            assert_eq!(value.to_string(), spelled, "{count} {unit}");
            // The round trip: the spelling read at the column's unit is the count.
            let read = Duration64::from_text(spelled).unwrap();
            assert_eq!(
                Scalar::Duration64(read).temporal_count_at(unit),
                Some(count),
                "{spelled}"
            );
            if let Ok(narrow) = i32::try_from(count) {
                let value = Duration32::new(narrow, unit, Timezone::NAIVE).unwrap();
                assert_eq!(value.to_string(), spelled, "{count} {unit} at 32 bits");
            }
        }
        // A day count has no classic spelling and keeps its structural form.
        assert!(
            Duration64::new(1, TimeUnit::Day, Timezone::NAIVE)
                .unwrap()
                .to_string()
                .starts_with("1@d["),
        );
    }
}
