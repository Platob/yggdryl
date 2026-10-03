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
