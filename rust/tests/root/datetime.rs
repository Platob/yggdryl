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
        assert_eq!(default.id().temporal_family(), Some("datetime"));
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
            DataType::from(default)
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
                assert_eq!(dtype, DataType::from(leaf));
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
            assert_refused(DataType::from(leaf).validate(), "datetime64", reason);
        }
    }

    #[test]
    fn every_timestamp_keyword_states_its_defaults_and_holds_to_what_it_states() {
        let at = |unit, timezone| DataType::datetime64(unit, timezone).unwrap();
        let paris = DataType::from_str("datetime64(us,\"Europe/Paris\")").unwrap();

        // Iceberg's three words - `timestamptz` PostgreSQL's and DuckDB's too,
        // `timestamp_ns` DuckDB's: a microsecond UTC instant, and the
        // nanosecond pair, whose unit is the word. The fold reads any case and any separator after `timestamptz`.
        for (source, expected) in [
            ("timestamptz", at(TimeUnit::Microsecond, Timezone::UTC)),
            ("TIMESTAMPTZ", at(TimeUnit::Microsecond, Timezone::UTC)),
            ("TimestampTz", at(TimeUnit::Microsecond, Timezone::UTC)),
            ("timestamptz(3)", at(TimeUnit::Millisecond, Timezone::UTC)),
            ("timestamptz(us, Europe/Paris)", paris.clone()),
            ("timestamptz(us, Some(Europe/Paris))", paris.clone()),
            (
                "timestamptz with time zone",
                at(TimeUnit::Microsecond, Timezone::UTC),
            ),
            ("timestamp_ns", at(TimeUnit::Nanosecond, Timezone::NAIVE)),
            ("TIMESTAMP_NS", at(TimeUnit::Nanosecond, Timezone::NAIVE)),
            ("timestampns", at(TimeUnit::Nanosecond, Timezone::NAIVE)),
            ("timestamptz_ns", at(TimeUnit::Nanosecond, Timezone::UTC)),
            ("timestamptz-ns", at(TimeUnit::Nanosecond, Timezone::UTC)),
            // The keywords that state a zone agree with a parameter that
            // states one too, and the one that states none with none.
            (
                "timestamp_ltz(us, Some(UTC))",
                at(TimeUnit::Microsecond, Timezone::UTC),
            ),
            (
                "timestamp_ntz(6)",
                at(TimeUnit::Microsecond, Timezone::NAIVE),
            ),
            (
                "timestamp_ntz(us, None)",
                at(TimeUnit::Microsecond, Timezone::NAIVE),
            ),
            (
                "timestamp_ntz without time zone",
                at(TimeUnit::Microsecond, Timezone::NAIVE),
            ),
            // `with time zone` agrees that there is a zone; a parameter that
            // named one said which, and it is kept.
            (
                "timestamptz(us, Europe/Paris) with time zone",
                paris.clone(),
            ),
            ("timestamp(us, Europe/Paris) with time zone", paris),
            // `timestamp` states nothing, so the suffix decides.
            (
                "timestamp with time zone",
                at(TimeUnit::Microsecond, Timezone::UTC),
            ),
            (
                "timestamp without time zone",
                at(TimeUnit::Microsecond, Timezone::NAIVE),
            ),
        ] {
            assert_eq!(DataType::from_str(source).unwrap(), expected, "{source}");
        }

        // `[]` after any timestamp keyword is the serie of it, never an empty
        // parameter list.
        for (source, item) in [
            ("timestamp_ns[]", "datetime64(ns)"),
            ("timestamptz[]", "datetime64(us,\"UTC\")"),
            ("timestamp[]", "datetime64(us)"),
            ("timestamptz(3)[]", "datetime64(ms,\"UTC\")"),
        ] {
            assert_eq!(
                DataType::from_str(source).unwrap(),
                DataType::from_str(&format!("serie<{item}>")).unwrap(),
                "{source}"
            );
        }

        // A word that states its unit takes no precision: one reading or none.
        for source in [
            "timestamp_ns(3)",
            "timestamp_ns(us)",
            "timestamptz_ns[9]",
            "timestamp_ns<ns>",
        ] {
            let error = DataType::from_str(source).unwrap_err().to_string();
            assert!(
                error.contains("which state their unit"),
                "{source}: {error}"
            );
        }

        // A zone statement that says the opposite of an earlier one - the
        // keyword's or a parameter's - is refused at the later statement,
        // naming where the earlier one stands.
        let zone = "expected a zone, as stated at byte";
        let none = "expected no zone, as stated at byte";
        for (source, position, reason) in [
            (
                "timestamptz without time zone",
                12,
                format!("{zone} 0, got none"),
            ),
            ("timestamptz(us, None)", 16, format!("{zone} 0, got none")),
            ("timestamptz(None)", 12, format!("{zone} 0, got none")),
            (
                "timestamp_ltz without time zone",
                14,
                format!("{zone} 0, got none"),
            ),
            (
                "timestamp_with_time_zone(us, None)",
                29,
                format!("{zone} 0, got none"),
            ),
            (
                "timestamp(us, UTC) without time zone",
                19,
                format!("{zone} 14, got none"),
            ),
            (
                "timestamp_ntz with time zone",
                14,
                format!("{none} 0, got one"),
            ),
            ("timestamp_ntz(us, UTC)", 18, format!("{none} 0, got one")),
            (
                "timestamp_ntz(us, Some(UTC))",
                18,
                format!("{none} 0, got one"),
            ),
            (
                "timestamp(us, None) with time zone",
                20,
                format!("{none} 14, got one"),
            ),
        ] {
            let error = DataType::from_str(source).unwrap_err().to_string();
            assert!(error.contains(&reason), "{source}: {error}");
            assert!(
                error.contains(&format!("at byte {position}:")),
                "{source}: {error}"
            );
        }
    }
}

/// `DateTime64::from_text`, the crate's one reader of datetime text.
mod from_text {
    use yggdryl::{DateTime64, TimeUnit, Timezone};

    /// 2026-10-03T03:20:00Z.
    const INSTANT: i64 = 1_790_997_600;

    fn read(text: &str, naive: Timezone) -> DateTime64 {
        DateTime64::from_text(text, naive).unwrap_or_else(|error| panic!("{text:?}: {error}"))
    }

    fn zone(name: &str) -> Timezone {
        Timezone::from_str(name).expect("a zone the registry knows")
    }

    #[test]
    fn every_spelling_of_an_instant_reads_as_that_instant_whatever_naive_says() {
        for (text, held) in [
            ("2026-10-03T03:20:00Z", Timezone::UTC),
            ("2026-10-03T03:20:00z", Timezone::UTC),
            ("2026-10-03T05:20:00+02:00", zone("+02:00")),
            ("2026-10-03T05:20:00+0200", zone("+02:00")),
            (
                "2026-10-03T05:20:00+02:00[Europe/Paris]",
                zone("Europe/Paris"),
            ),
        ] {
            for naive in [Timezone::UTC, Timezone::NAIVE, zone("Asia/Tokyo")] {
                let at = read(text, naive);
                assert_eq!(
                    (at.count(), at.unit()),
                    (INSTANT, TimeUnit::Second),
                    "{text:?}"
                );
                assert_eq!(at.timezone(), held, "{text:?}: the zone the text states");
            }
        }
    }

    #[test]
    fn a_reading_that_states_no_zone_is_a_wall_clock_in_the_zone_given() {
        let utc = read("2026-10-03 03:20:00", Timezone::UTC);
        assert_eq!((utc.count(), utc.timezone()), (INSTANT, Timezone::UTC));
        let wall = read("2026-10-03T03:20:00", Timezone::NAIVE);
        assert_eq!(
            (wall.count(), wall.timezone()),
            (INSTANT, Timezone::NAIVE),
            "a naive reading stays a wall clock, its count the clock's own"
        );
        // Paris keeps summer time on the 3rd of October and winter time in
        // January: the zone's rules, not a fixed offset, place the clock.
        let paris = zone("Europe/Paris");
        let summer = read("2026-10-03 05:20:00", paris);
        assert_eq!((summer.count(), summer.timezone()), (INSTANT, paris));
        assert_eq!(read("2026-01-05T10:00:00", paris).count(), 1_767_603_600);
        let midnight = read("2026-10-03", Timezone::UTC);
        assert_eq!(
            (midnight.count(), midnight.unit()),
            (1_790_985_600, TimeUnit::Second),
            "a bare date is that day's midnight"
        );
    }

    #[test]
    fn the_resolution_is_the_one_the_digits_spell() {
        for (text, count, unit) in [
            ("2026-10-03T03:20:00Z", INSTANT, TimeUnit::Second),
            (
                "2026-10-03T03:20:00.250Z",
                INSTANT * 1_000 + 250,
                TimeUnit::Millisecond,
            ),
            (
                "2026-10-03 03:20:00.000250",
                INSTANT * 1_000_000 + 250,
                TimeUnit::Microsecond,
            ),
            (
                "2026-10-03T03:20:00.000000250Z",
                INSTANT * 1_000_000_000 + 250,
                TimeUnit::Nanosecond,
            ),
        ] {
            let at = read(text, Timezone::UTC);
            assert_eq!((at.count(), at.unit()), (count, unit), "{text:?}");
        }
    }

    #[test]
    fn it_reads_nothing_the_iso_readers_do_not_so_a_cell_and_the_value_door_agree() {
        // Blanks around the text and a tool's trailing `UTC` are the habits
        // of one intake - an expiry's, `auth::instant` - and no part of the
        // reading: the value door of a datetime refuses them, and a cell
        // read through here must refuse them with it.
        for text in [
            " 2026-10-03",
            "2026-10-03 ",
            " 2026-10-03T03:20:00Z",
            "2026-10-03T03:20:00Z\n",
            "2026-10-03T03:20:00UTC",
            "2026-10-03T03:20:00 UTC",
        ] {
            for naive in [Timezone::UTC, Timezone::NAIVE] {
                assert!(
                    DateTime64::from_text(text, naive).is_err(),
                    "{text:?} is refused, as the value door refuses it"
                );
            }
        }
    }

    #[test]
    fn text_that_is_no_datetime_is_refused() {
        for text in [
            "",
            "soon",
            "2026-13-03T03:20:00Z",
            "2026-10-03T03:20:00Z trailing",
            "UTC",
        ] {
            assert!(
                DateTime64::from_text(text, Timezone::UTC).is_err(),
                "{text:?} is no datetime"
            );
        }
    }
}

/// The short spelling every datetime renders through: no fraction where it
/// is zero, the shortest exact one otherwise, the zone suffix unchanged.
mod text {
    use yggdryl::{DateTime64, Scalar, TimeUnit, Timezone};

    /// 2026-08-14T14:52:55Z.
    const INSTANT: i64 = 1_786_719_175;

    fn zone(name: &str) -> Timezone {
        Timezone::from_str(name).expect("a zone the registry knows")
    }

    #[test]
    fn a_datetime_prints_no_fraction_where_it_is_zero_and_the_shortest_exact_one_otherwise() {
        let per = |unit: TimeUnit| match unit {
            TimeUnit::Second => 1,
            TimeUnit::Millisecond => 1_000,
            TimeUnit::Microsecond => 1_000_000,
            _ => 1_000_000_000,
        };
        // The fractions a spelling may carry, in nanoseconds, and the digits
        // each spells: nothing, three, six, nine.
        let fractions = [
            (0, ""),
            (500_000_000, ".500"),
            (1_000, ".000001"),
            (1, ".000000001"),
        ];
        // The zones, and the suffix each writes after the local reading - the
        // 14th of August is summer time in Zurich.
        let zones = [
            (Timezone::UTC, "2026-08-14T14:52:55", "Z"),
            (Timezone::NAIVE, "2026-08-14T14:52:55", ""),
            (zone("+02:00"), "2026-08-14T16:52:55", "+02:00"),
            (
                zone("Europe/Zurich"),
                "2026-08-14T16:52:55",
                "+02:00[Europe/Zurich]",
            ),
            (zone("-08:00"), "2026-08-14T06:52:55", "-08:00"),
        ];
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            for (nanos, digits) in fractions {
                // A fraction the unit cannot hold is not a count at it.
                if nanos % (1_000_000_000 / per(unit)) != 0 {
                    continue;
                }
                let count = INSTANT * per(unit) + nanos / (1_000_000_000 / per(unit));
                for (held, local, suffix) in &zones {
                    let value = DateTime64::new(count, unit, *held).unwrap();
                    let spelled = format!("{local}{digits}{suffix}");
                    assert_eq!(value.to_string(), spelled, "{count} {unit} {held}");
                    // The round trip: the spelling read in the column's zone
                    // and restated at its unit is the count, and the zone is
                    // the one the text states or, stating none, the column's.
                    let read = DateTime64::from_text(&spelled, *held).unwrap();
                    assert_eq!(
                        Scalar::DateTime64(read).temporal_count_at(unit),
                        Some(count),
                        "{spelled}"
                    );
                    assert_eq!(read.timezone(), *held, "{spelled}");
                }
            }
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The two FIX doors, which the FIX codec alone reaches.

    use yggdryl::internals::datetime::{from_fix_clock, from_fix_text};
    use yggdryl::{Error, TimeUnit, Timezone};

    fn zone(name: &str) -> Timezone {
        Timezone::from_str(name).expect("a zone the registry knows")
    }

    #[test]
    fn the_clock_door_refuses_a_dated_value_and_what_is_no_clock() {
        // The refusal first: a `TZTimeOnly` states a clock and no date, so a
        // date is refused by its shape and the rest naming the byte.
        for (text, position) in [
            ("20060901-07:39Z", 0),
            ("20060901-07:39:12", 0),
            ("20260930", 6),
            ("7:39Z", 0),
            ("07:60Z", 0),
            ("07:39:61", 0),
            ("07:39:12 Z", 8),
            ("0930001", 6),
            ("", 0),
            ("soon", 0),
        ] {
            let error = from_fix_clock(text, Timezone::UTC).unwrap_err();
            assert!(
                matches!(&error, Error::Parse { position: held, .. } if *held == position),
                "{text:?}: {error}"
            );
        }
        // An extended date opens as a minute clock and breaks at the zone
        // its `-` is taken for; whatever the byte, it is no clock.
        assert!(from_fix_clock("2026-09-30", Timezone::UTC).is_err());
        // The datetime door keeps refusing a clock stating neither a date
        // nor a zone: a `TZTimeOnly` is the one field that reads one.
        for text in ["07:39:12", "07:39", "093000"] {
            assert!(from_fix_text(text, Timezone::UTC).is_err(), "{text:?}");
        }
    }

    #[test]
    fn the_clock_door_reads_a_clock_on_the_epoch_day_zoned_by_the_text_or_the_column() {
        // A stated zone resolves into the instant, which the offset may
        // carry behind the epoch; the zone held is the text's.
        for (text, count, unit, held) in [
            (
                "07:39:12.123+05:30",
                7_752_123,
                TimeUnit::Millisecond,
                "+05:30",
            ),
            ("07:39Z", 27_540, TimeUnit::Second, "UTC"),
            ("0739Z", 27_540, TimeUnit::Second, "UTC"),
            ("00:30+05:30", -18_000, TimeUnit::Second, "+05:30"),
            ("07:39:12 +0530", 7_752, TimeUnit::Second, "+05:30"),
            (
                "093000123+05:30",
                14_400_123,
                TimeUnit::Millisecond,
                "+05:30",
            ),
        ] {
            for naive in [Timezone::UTC, Timezone::NAIVE, zone("Asia/Tokyo")] {
                let read =
                    from_fix_clock(text, naive).unwrap_or_else(|error| panic!("{text:?}: {error}"));
                assert_eq!((read.count(), read.unit()), (count, unit), "{text:?}");
                assert_eq!(read.timezone(), zone(held), "{text:?}");
            }
        }
        // A clock stating no zone is the wall clock in the zone given, on
        // the epoch day: in UTC the count is the clock itself.
        for (text, count, unit) in [
            ("093000", 34_200, TimeUnit::Second),
            ("09:30:00", 34_200, TimeUnit::Second),
            ("09:30", 34_200, TimeUnit::Second),
            ("0930", 34_200, TimeUnit::Second),
            ("093000123", 34_200_123, TimeUnit::Millisecond),
            (
                "09:30:00.000000001",
                34_200_000_000_001,
                TimeUnit::Nanosecond,
            ),
            // An hour past the day carries into the next, as a datetime's does.
            ("25:00:00", 90_000, TimeUnit::Second),
        ] {
            let utc = from_fix_clock(text, Timezone::UTC)
                .unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(
                (utc.count(), utc.unit(), utc.timezone()),
                (count, unit, Timezone::UTC),
                "{text:?}"
            );
            let wall = from_fix_clock(text, Timezone::NAIVE).unwrap();
            assert_eq!(
                (wall.count(), wall.timezone()),
                (count, Timezone::NAIVE),
                "{text:?}"
            );
        }
        // In a place zone the wall clock is placed by the zone's rules on
        // the epoch day: Kolkata was five and a half hours ahead.
        let kolkata = from_fix_clock("093000", zone("Asia/Kolkata")).unwrap();
        assert_eq!(
            (kolkata.count(), kolkata.timezone()),
            (34_200 - 19_800, zone("Asia/Kolkata"))
        );
        // The typed `TZTimeOnly` prints as the instant it is.
        assert_eq!(
            from_fix_clock("093000", Timezone::UTC).unwrap().to_string(),
            "1970-01-01T09:30:00Z"
        );
    }

    #[test]
    fn the_datetime_door_stands_as_it_was() {
        // 2026-08-14T14:52:55Z.
        let at = from_fix_text("20260814-14:52:55", Timezone::UTC).unwrap();
        assert_eq!(
            (at.count(), at.unit(), at.timezone()),
            (1_786_719_175, TimeUnit::Second, Timezone::UTC)
        );
        assert_eq!(at.to_string(), "2026-08-14T14:52:55Z");
        // A bare date under a naive column is that day's midnight, a wall clock.
        let day = from_fix_text("20260930", Timezone::NAIVE).unwrap();
        assert_eq!(
            (day.count(), day.timezone()),
            (20_726 * 86_400, Timezone::NAIVE)
        );
        assert_eq!(day.to_string(), "2026-09-30T00:00:00");
        // The digit run, the minute clock, the zoned dateless clock and the
        // closed offset read as `rust/tests/root/temporal.rs` pins them.
        assert_eq!(
            from_fix_text("20240102101530123", Timezone::UTC)
                .unwrap()
                .count(),
            1_704_190_530_123
        );
        assert_eq!(
            from_fix_text("20060901-07:39Z", Timezone::NAIVE)
                .unwrap()
                .count(),
            1_157_096_340
        );
        assert_eq!(
            from_fix_text("07:39Z", Timezone::NAIVE).unwrap().count(),
            27_540
        );
        assert_eq!(
            from_fix_text("20260101-10:00:00 +0400s", Timezone::UTC)
                .unwrap()
                .count(),
            1_767_247_200
        );
    }
}
