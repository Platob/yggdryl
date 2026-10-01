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
