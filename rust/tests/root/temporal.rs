//! `rust/src/temporal.rs`: the ISO 8601 readers and writers no caller can name.

use super::typed;

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::temporal::{
        format_date, format_datetime, format_duration, format_time, format_timestamp,
        from_temporal_text, parse_date, parse_datetime, parse_duration, parse_time,
        parse_timestamp,
    };
    use yggdryl::{Error, TimeUnit, Timezone};

    /// The empty text is no spelling: the empty-cell rule sits above this
    /// reader, on the doors, and the reader itself keeps refusing it.
    #[test]
    fn the_empty_text_is_no_temporal_spelling() {
        use yggdryl::{DataType, TimeUnit, Timezone};

        for dtype in [
            DataType::date32(),
            DataType::time64(TimeUnit::Microsecond).unwrap(),
            DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
        ] {
            assert!(from_temporal_text(&dtype, "").is_err(), "{dtype}");
        }
    }

    #[test]
    fn dates_round_trip_and_reject_days_that_do_not_exist() {
        assert_eq!(format_date(0).as_deref(), Some("1970-01-01"));
        assert_eq!(format_date(20_682).as_deref(), Some("2026-08-17"));
        assert_eq!(format_date(-719_162).as_deref(), Some("0001-01-01"));

        assert_eq!(parse_date("2026-08-17").unwrap(), 20_682);
        assert_eq!(parse_date("1970-01-01").unwrap(), 0);
        // A leap day parses in a leap year and errors in any other.
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2026-02-29").is_err());
        assert!(parse_date("2026-13-01").is_err());
        assert!(parse_date("2026-08-17T").is_err());

        // A five-digit year has no classic spelling, so it keeps its structure.
        assert_eq!(format_date(i32::MAX), None);
    }

    #[test]
    fn times_print_the_fraction_at_the_unit_width() {
        assert_eq!(
            format_time(0, TimeUnit::Second).as_deref(),
            Some("00:00:00")
        );
        assert_eq!(
            format_time(36_000 + 23 * 60 + 45, TimeUnit::Second).as_deref(),
            Some("10:23:45")
        );
        assert_eq!(
            format_time(36_001_500, TimeUnit::Millisecond).as_deref(),
            Some("10:00:01.500")
        );
        assert_eq!(
            format_time(1, TimeUnit::Nanosecond).as_deref(),
            Some("00:00:00.000000001")
        );

        // The digit count is the unit on the way back.
        assert_eq!(
            parse_time("10:00:01.500").unwrap(),
            (36_001_500, TimeUnit::Millisecond)
        );
        assert_eq!(parse_time("10:23:45").unwrap(), (37_425, TimeUnit::Second));
        assert_eq!(
            parse_time("00:00:00.5").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00.000001").unwrap(),
            (1, TimeUnit::Microsecond)
        );

        // A reading outside its day has no clock spelling.
        assert_eq!(format_time(-1, TimeUnit::Second), None);
        assert_eq!(format_time(86_400, TimeUnit::Second), None);
    }

    #[test]
    fn a_time_of_day_folds_an_hour_past_the_end_of_its_day() {
        // Midnight closing a shift is the midnight that opens the next day.
        assert_eq!(parse_time("24:00:00").unwrap(), (0, TimeUnit::Second));
        assert_eq!(parse_time("25:30:00").unwrap(), (5_400, TimeUnit::Second));
        // Hours run to the two digits the field holds: 99:59:59 is 03:59:59.
        assert_eq!(parse_time("99:59:59").unwrap(), (14_399, TimeUnit::Second));
        assert!(parse_time("100:00:00").is_err());

        // The fold is on the count, so it keeps the fraction and its unit.
        assert_eq!(
            parse_time("24:00:00.500").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("48:00:00.000_001").unwrap(),
            (1, TimeUnit::Microsecond)
        );

        // Minutes and seconds are calendar fields and stay under sixty.
        assert!(parse_time("10:60:00").is_err());
        assert!(parse_time("10:00:60").is_err());

        // Folding is not round-tripping: the spelling comes back inside the day.
        assert_eq!(
            format_time(parse_time("25:30:00").unwrap().0, TimeUnit::Second).as_deref(),
            Some("01:30:00")
        );
    }

    #[test]
    fn a_datetime_carries_an_hour_past_the_end_of_its_day() {
        // A datetime is a point on the line, so the hour carries into the date.
        assert_eq!(
            parse_datetime("2026-08-17T24:00:00").unwrap(),
            parse_datetime("2026-08-18T00:00:00").unwrap()
        );
        assert_eq!(
            parse_datetime("2026-08-17T25:30:00.250").unwrap(),
            parse_datetime("2026-08-18T01:30:00.250").unwrap()
        );
        // Across a month end, the carry uses the calendar rather than a guess.
        assert_eq!(
            parse_datetime("2026-02-28T30:00:00").unwrap(),
            parse_datetime("2026-03-01T06:00:00").unwrap()
        );
        assert_eq!(
            parse_timestamp("2026-08-17T24:00:00Z").unwrap(),
            parse_timestamp("2026-08-18T00:00:00Z").unwrap()
        );
    }

    #[test]
    fn a_date_with_no_clock_reads_as_that_day_at_midnight() {
        // A wire that spells a settlement date spells the day and stops, so the
        // day is the instant it opens - the compact spelling FIX writes and the
        // extended one a bridge quotes it with are one reading.
        let midnight = parse_datetime("2026-08-18T00:00:00").unwrap();
        assert_eq!(midnight, (1_787_011_200, TimeUnit::Second));
        assert_eq!(parse_datetime("2026-08-18").unwrap(), midnight);
        assert_eq!(parse_datetime("20260818").unwrap(), midnight);
        assert_eq!(parse_datetime("19700101").unwrap(), (0, TimeUnit::Second));
        assert_eq!(
            parse_datetime("19691231").unwrap(),
            (-86_400, TimeUnit::Second)
        );

        // The day is still the calendar's, and the date reader is the one that
        // says so: February 30th is no date and therefore no datetime.
        assert!(parse_datetime("20260230").is_err());
        assert!(parse_datetime("2026-02-30").is_err());
        assert!(parse_datetime("20261301").is_err());

        // A zoned reading states its zone and then reads the same midnight, in
        // either case of the `Z` that states UTC.
        let (count, unit, zone) = parse_timestamp("20260818Z").unwrap();
        assert_eq!((count, unit), midnight);
        assert!(zone.is_utc());
        assert_eq!(parse_timestamp("20260818z").unwrap().0, midnight.0);
        let (count, _, zone) = parse_timestamp("2026-08-18+02:00[Europe/Paris]").unwrap();
        assert_eq!(count, midnight.0 - 2 * 3_600);
        assert_eq!(zone, Timezone::from_str("Europe/Paris").unwrap());
        assert_eq!(
            parse_timestamp("2026-08-18+02:00").unwrap().0,
            midnight.0 - 2 * 3_600
        );

        // The zone is required in the text and nowhere else: a bare date is a
        // naive reading, and a naive reading carries no zone.
        assert!(parse_timestamp("20260818").is_err());
        assert!(parse_timestamp("2026-08-18[Europe/Paris]").is_err());
        assert!(parse_datetime("20260818Z").is_err());

        // A date is read as a datetime and never written as one: the count spells
        // its clock back.
        assert_eq!(
            format_datetime(midnight.0, TimeUnit::Second).as_deref(),
            Some("2026-08-18T00:00:00")
        );

        // The other readers keep their own shapes. A date is a date, a clock is
        // not a date, and neither reads the other's trailing text.
        assert_eq!(parse_date("20260818").unwrap(), 20_683);
        assert!(parse_date("2026-08-18T00:00:00").is_err());
        assert!(parse_time("20260818").is_err());
        assert!(parse_duration("20260818").is_err());
    }

    #[test]
    fn a_date_that_opens_a_clock_still_has_to_finish_it() {
        let position = |text: &str| match parse_datetime(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "datetime");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // A separator opens a clock, so it is followed by one or by nothing at
        // all: the byte the refusal names is the one the clock was owed at.
        assert_eq!(position("2026-08-18T"), 11);
        assert_eq!(position("20260818T"), 9);
        assert_eq!(position("2026-08-18 "), 11);
        // Trailing text that opens nothing names where the date ended instead.
        assert_eq!(position("2026-08-18X"), 10);

        // `-` after a date is FIX's clock separator, which is why a bare date
        // takes `Z` or a `+` offset and never a `-` one: a clock that stops at
        // its minutes is a broken clock rather than a western offset.
        assert_eq!(
            parse_datetime("20260818-10:15:00").unwrap(),
            parse_datetime("2026-08-18T10:15:00").unwrap()
        );
        assert!(parse_timestamp("20260818-08:00").is_err());
        assert!(parse_timestamp("2026-08-18-08:00").is_err());

        // A digit run is a date at eight and a date and a clock at fourteen, and
        // nothing at the widths between, where a field would be half written.
        for held in [
            "2026",
            "202608",
            "2026081",
            "202608181",
            "2026081810",
            "202608181015",
            "2026081810153",
            "202608181015300",
        ] {
            assert!(parse_datetime(held).is_err(), "{held}");
        }
        assert_eq!(
            parse_datetime("20260818101530").unwrap(),
            parse_datetime("2026-08-18T10:15:30").unwrap()
        );
    }

    #[test]
    fn fraction_separators_group_digits_without_changing_the_count() {
        // `_` groups digits; the count and the unit are exactly the ungrouped ones.
        assert_eq!(
            parse_time("10:00:00.000_000").unwrap(),
            parse_time("10:00:00.000000").unwrap()
        );
        assert_eq!(
            parse_time("10:00:00.000_001").unwrap(),
            (36_000_000_001, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_time("00:00:00.1_2_3").unwrap(),
            (123, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00.123_456_789").unwrap(),
            (123_456_789, TimeUnit::Nanosecond)
        );
        assert_eq!(
            parse_datetime("2024-02-01 10:00:00.000_000").unwrap(),
            parse_datetime("2024-02-01T10:00:00.000000").unwrap()
        );
        let (count, unit, zone) = parse_timestamp("2024-02-01T10:00:00.500_000Z").unwrap();
        assert_eq!(
            (count, unit),
            (1_706_781_600_500_000, TimeUnit::Microsecond)
        );
        assert!(zone.is_utc());

        // The grouped digits still count toward the 1-to-9 budget.
        assert!(parse_time("00:00:00.000_000_000_1").is_err());
    }

    #[test]
    fn either_decimal_sign_opens_the_same_fraction() {
        // ISO 8601 names the comma and the full stop alike and prefers the comma,
        // which is what a log4j line and a European locale write; the count, the
        // unit and the grouping rule are the ones the full stop already had.
        assert_eq!(
            parse_time("10:00:01,500").unwrap(),
            parse_time("10:00:01.500").unwrap()
        );
        assert_eq!(
            parse_time("00:00:00,5").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00,000_001").unwrap(),
            (1, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_datetime("2026-08-14 00:05:01,148").unwrap(),
            parse_datetime("2026-08-14T00:05:01.148").unwrap()
        );
        let (count, unit, zone) = parse_timestamp("2024-02-01T10:00:00,500_000Z").unwrap();
        assert_eq!(
            (count, unit),
            (1_706_781_600_500_000, TimeUnit::Microsecond)
        );
        assert!(zone.is_utc());

        // The sign is read, never written: a comma comes back as the full stop
        // RFC 3339 allows, so a spelling normalizes on the way out.
        assert_eq!(
            format_time(parse_time("00:00:00,5").unwrap().0, TimeUnit::Millisecond).as_deref(),
            Some("00:00:00.500")
        );

        // One sign, not two: the second is trailing text, not more fraction.
        assert!(parse_time("00:00:00,5.5").is_err());
        assert!(parse_time("00:00:00.5,5").is_err());
    }

    #[test]
    fn malformed_fraction_separators_are_rejected_with_their_byte_position() {
        let position = |text: &str| match parse_time(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "time");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // Leading, trailing, doubled, and lone separators each name the byte of
        // the `_` that breaks the digit-grouping rule.
        assert_eq!(position("00:00:00._5"), 9);
        assert_eq!(position("00:00:00.5_"), 10);
        // In a doubled pair the first `_` is already not between digits.
        assert_eq!(position("00:00:00.5__5"), 10);
        assert_eq!(position("00:00:00._"), 9);
        assert_eq!(position("00:00:00.1_2_"), 12);

        // The comma opens a fraction exactly where the full stop does, so a
        // malformed one names the same byte.
        assert_eq!(position("00:00:00,"), 9);
        assert_eq!(position("00:00:00,_5"), 9);
        assert_eq!(position("00:00:00,1_2_"), 12);

        // The same clock feeds the datetime and timestamp parsers.
        assert!(parse_datetime("2024-02-01T10:00:00._5").is_err());
        assert!(parse_timestamp("2024-02-01T10:00:00.5_Z").is_err());
    }

    #[test]
    fn naive_datetimes_split_the_epoch_count_exactly() {
        assert_eq!(
            format_datetime(1_700_000_000, TimeUnit::Second).as_deref(),
            Some("2023-11-14T22:13:20")
        );
        assert_eq!(
            format_datetime(-1, TimeUnit::Second).as_deref(),
            Some("1969-12-31T23:59:59")
        );
        assert_eq!(
            parse_datetime("2023-11-14T22:13:20").unwrap(),
            (1_700_000_000, TimeUnit::Second)
        );
        assert_eq!(
            parse_datetime("2023-11-14 22:13:20.250").unwrap(),
            (1_700_000_000_250, TimeUnit::Millisecond)
        );
        // A naive reading carries no zone, and saying one is an error.
        assert!(parse_datetime("2023-11-14T22:13:20Z").is_err());
    }

    #[test]
    fn zoned_instants_spell_the_local_reading_and_recover_the_instant() {
        let utc = Timezone::UTC;
        assert_eq!(
            format_timestamp(1_700_000_000, TimeUnit::Second, &utc).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );

        let kolkata = Timezone::from_str("Asia/Kolkata").unwrap();
        assert_eq!(
            format_timestamp(0, TimeUnit::Second, &kolkata).as_deref(),
            Some("1970-01-01T05:30:00+05:30[Asia/Kolkata]")
        );

        let fixed = Timezone::from_str("-08:00").unwrap();
        assert_eq!(
            format_timestamp(3_600_000, TimeUnit::Millisecond, &fixed).as_deref(),
            Some("1969-12-31T17:00:00.000-08:00")
        );

        // The offset recovers the instant; the bracket recovers the name.
        let (count, unit, zone) =
            parse_timestamp("1970-01-01T05:30:00+05:30[Asia/Kolkata]").unwrap();
        assert_eq!((count, unit), (0, TimeUnit::Second));
        assert_eq!(zone, kolkata);

        let (count, unit, zone) = parse_timestamp("2023-11-14T22:13:20Z").unwrap();
        assert_eq!((count, unit), (1_700_000_000, TimeUnit::Second));
        assert!(zone.is_utc());

        let (count, _, zone) = parse_timestamp("1969-12-31T17:00:00.000-08:00").unwrap();
        assert_eq!(count, 3_600_000);
        assert_eq!(zone, fixed);

        // A zone with no rules in this build keeps the instant in UTC and the
        // name in the bracket.
        let unknown = Timezone::from_str("Mars/Olympus").unwrap();
        let spelled = format_timestamp(60, TimeUnit::Second, &unknown).unwrap();
        assert_eq!(spelled, "1970-01-01T00:01:00Z[Mars/Olympus]");
        let (count, _, zone) = parse_timestamp(&spelled).unwrap();
        assert_eq!(count, 60);
        assert_eq!(zone, unknown);

        assert!(parse_timestamp("2023-11-14T22:13:20").is_err());
    }

    #[test]
    fn durations_spell_seconds_and_read_any_decomposition() {
        assert_eq!(
            format_duration(90, TimeUnit::Second).as_deref(),
            Some("PT90S")
        );
        assert_eq!(
            format_duration(1_500, TimeUnit::Millisecond).as_deref(),
            Some("PT1.500S")
        );
        assert_eq!(
            format_duration(-1_500, TimeUnit::Millisecond).as_deref(),
            Some("-PT1.500S")
        );
        assert_eq!(format_duration(1, TimeUnit::YearMonth), None);

        assert_eq!(parse_duration("PT90S").unwrap(), (90, TimeUnit::Second));
        assert_eq!(
            parse_duration("PT1.500S").unwrap(),
            (1_500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_duration("-PT1.5S").unwrap(),
            (-1_500, TimeUnit::Millisecond)
        );
        // The decomposed general form restates in seconds.
        assert_eq!(
            parse_duration("P1DT2H3M4S").unwrap(),
            (86_400 + 2 * 3_600 + 3 * 60 + 4, TimeUnit::Second)
        );
        assert!(parse_duration("P").is_err());
        assert!(parse_duration("PT1.5M").is_err());

        // The shared fraction reader means the duration spellings take the comma
        // too, as ISO 8601 says they do.
        assert_eq!(
            parse_duration("PT1,5S").unwrap(),
            parse_duration("PT1.5S").unwrap()
        );
        assert!(parse_duration("PT1,5M").is_err());
    }

    #[test]
    fn durations_also_read_a_plain_clock_whose_hours_never_fold() {
        // The hours are elapsed hours, so they stay whole where a time of day
        // would fold: 25:00:00 is twenty-five hours, not one o'clock.
        assert_eq!(
            parse_duration("01:30:00").unwrap(),
            (5_400, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("25:00:00").unwrap(),
            (90_000, TimeUnit::Second)
        );
        assert_eq!(parse_duration("00:00:00").unwrap(), (0, TimeUnit::Second));
        // A count takes the width it needs, and the sign leads it.
        assert_eq!(
            parse_duration("1:00:00").unwrap(),
            (3_600, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("100:00:00").unwrap(),
            (360_000, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("-01:30:00").unwrap(),
            (-5_400, TimeUnit::Second)
        );
        assert_eq!(parse_duration("+00:00:01").unwrap(), (1, TimeUnit::Second));

        // The fraction rules are the clock's own, grouping separators included.
        assert_eq!(
            parse_duration("00:00:01.500").unwrap(),
            (1_500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_duration("-00:01:00.000_001").unwrap(),
            (-60_000_001, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_duration("00:00:00.5").unwrap(),
            parse_duration("PT0.5S").unwrap()
        );
        assert_eq!(
            parse_duration("25:30:00,5").unwrap(),
            parse_duration("25:30:00.5").unwrap()
        );
        // Both spellings of the same elapsed time read the same count.
        assert_eq!(
            parse_duration("26:03:04").unwrap(),
            parse_duration("P1DT2H3M4S").unwrap()
        );

        // Minutes and seconds keep the clock's two-digit fields under sixty.
        assert!(parse_duration("01:60:00").is_err());
        assert!(parse_duration("01:00:60").is_err());
        assert!(parse_duration("01:0:00").is_err());
        assert!(parse_duration("01:30").is_err());
        assert!(parse_duration(":30:00").is_err());
        assert!(parse_duration("01:30:00Z").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("later").is_err());
        // An hour count beyond the unit's reach is out of range, not a wrap.
        assert!(parse_duration("9999999999:00:00.000000001").is_err());
        assert!(parse_duration("99999999999999999999:00:00").is_err());
    }

    #[test]
    fn malformed_durations_name_the_byte_that_breaks_them() {
        let position = |text: &str| match parse_duration(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "duration");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // Positions are byte offsets into the text as written, sign included.
        assert_eq!(position("-PT1.5M"), 6);
        assert_eq!(position("PT1.5M"), 5);
        // The comma reaches the component labels through the same fraction, so
        // the byte a fraction on the wrong component names moves with it.
        assert_eq!(position("PT1,5M"), 5);
        assert_eq!(position("P1,5D"), 4);
        assert_eq!(position("P1Y"), 2);
        assert_eq!(position("PT1"), 3);
        assert_eq!(position("-01:60:00"), 4);
        assert_eq!(position("-x"), 1);
    }
}

mod datatypes {
    use arrow_schema::{
        DataType as ArrowDataType, IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit,
    };
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    #[test]
    fn every_temporal_leaf_projects_to_its_arrow_storage_and_imports_back() {
        // Each family lays out its own storage, and the storage imports as the
        // leaf that declared it.
        let cases = [
            (DataType::date32(), ArrowDataType::Date32),
            (DataType::date64(), ArrowDataType::Date64),
            (
                DataType::time32(TimeUnit::Second).unwrap(),
                ArrowDataType::Time32(ArrowTimeUnit::Second),
            ),
            (
                DataType::time32(TimeUnit::Millisecond).unwrap(),
                ArrowDataType::Time32(ArrowTimeUnit::Millisecond),
            ),
            (
                DataType::time64(TimeUnit::Microsecond).unwrap(),
                ArrowDataType::Time64(ArrowTimeUnit::Microsecond),
            ),
            (
                DataType::time64(TimeUnit::Nanosecond).unwrap(),
                ArrowDataType::Time64(ArrowTimeUnit::Nanosecond),
            ),
            (
                DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
                ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, None),
            ),
            (
                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
                ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, Some("UTC".into())),
            ),
            (
                DataType::duration64(TimeUnit::Millisecond).unwrap(),
                ArrowDataType::Duration(ArrowTimeUnit::Millisecond),
            ),
            (
                DataType::interval(TimeUnit::YearMonth).unwrap(),
                ArrowDataType::Interval(ArrowIntervalUnit::YearMonth),
            ),
            (
                DataType::interval(TimeUnit::DayTime).unwrap(),
                ArrowDataType::Interval(ArrowIntervalUnit::DayTime),
            ),
            (
                DataType::interval(TimeUnit::MonthDayNano).unwrap(),
                ArrowDataType::Interval(ArrowIntervalUnit::MonthDayNano),
            ),
        ];
        for (dtype, storage) in cases {
            assert_eq!(dtype.to_arrow_datatype().unwrap(), storage, "{dtype}");
            assert_eq!(
                dtype.clone().into_arrow_datatype().unwrap(),
                storage,
                "{dtype}"
            );
            assert_eq!(
                DataType::from_arrow_datatype(&storage).unwrap(),
                dtype,
                "{dtype}"
            );
        }

        // Arrow has one duration width, so the 32-bit leaf widens on the way
        // out and comes back as the 64-bit one.
        assert_eq!(
            DataType::duration32(TimeUnit::Second)
                .unwrap()
                .to_arrow_datatype()
                .unwrap(),
            ArrowDataType::Duration(ArrowTimeUnit::Second)
        );
        assert_eq!(
            DataType::from_arrow_datatype(&ArrowDataType::Duration(ArrowTimeUnit::Second)).unwrap(),
            DataType::duration64(TimeUnit::Second).unwrap()
        );

        // A hand-built leaf its width does not carry stops at the boundary
        // rather than widening silently.
        for dtype in [
            DataType::Time32(TimeUnit::Nanosecond),
            DataType::Time64(TimeUnit::Second),
            DataType::DateTime64 {
                unit: TimeUnit::Day,
                timezone: Timezone::NAIVE,
            },
            DataType::Duration32(TimeUnit::YearMonth),
            DataType::Interval(TimeUnit::Second),
        ] {
            assert!(dtype.to_arrow_datatype().is_err(), "{dtype}");
            assert!(dtype.into_arrow_datatype().is_err());
        }
    }

    #[test]
    fn a_temporal_reads_the_compact_spelling_and_the_extended_one_alike() {
        let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let naive = DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();

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
            DataType::date32().scalar(Scalar::from("20240102")).unwrap(),
            DataType::date32()
                .scalar(Scalar::from("2024-01-02"))
                .unwrap(),
        );
        let nanos = DataType::time64(TimeUnit::Nanosecond).unwrap();
        assert_eq!(
            nanos.scalar(Scalar::from("101530.5")).unwrap(),
            nanos.scalar(Scalar::from("10:15:30.5")).unwrap(),
        );

        // A date states no clock, so a datetime column reads one as that day's
        // midnight - which is how a settlement date sits in the column a transact
        // time sits in, rather than in a second one to cast through.
        let midnight = naive
            .scalar(Scalar::from("2024-01-02T00:00:00"))
            .expect("the extended midnight");
        for held in ["2024-01-02", "20240102"] {
            assert_eq!(
                naive.scalar(Scalar::from(held)).unwrap(),
                midnight,
                "{held}"
            );
        }

        // A zoned column still wants the zone stated, and reads the same midnight
        // once it is; a date that states none is a naive reading and stays one.
        let zoned_midnight = utc
            .scalar(Scalar::from("2024-01-02T00:00:00Z"))
            .expect("the extended midnight");
        for held in ["2024-01-02Z", "20240102Z", "20240102+00:00"] {
            assert_eq!(
                utc.scalar(Scalar::from(held)).unwrap(),
                zoned_midnight,
                "{held}"
            );
        }
        assert!(utc.scalar(Scalar::from("20240102")).is_err());
        assert!(naive.scalar(Scalar::from("20240102Z")).is_err());

        // What is not a spelling of anything is still refused, at its position.
        for held in ["2024010210153", "202401021015300", "20241302101530"] {
            assert!(naive.scalar(Scalar::from(held)).is_err(), "{held}");
        }
    }

    #[test]
    fn either_decimal_sign_reads_the_same_fraction() {
        let naive = DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE).unwrap();

        // ISO 8601 divides a fraction from its integer part with either decimal
        // sign and names the comma the preferred one, so a log4j row and a
        // European locale's clock read the instant their dotted twin reads - in
        // every spelling of the datetime that carries them.
        let stated = naive
            .scalar(Scalar::from("2026-08-14T00:05:01.148"))
            .expect("the full stop");
        for held in [
            "2026-08-14T00:05:01,148",
            "2026-08-14 00:05:01,148",
            "20260814-00:05:01,148",
        ] {
            assert_eq!(naive.scalar(Scalar::from(held)).unwrap(), stated, "{held}");
        }

        // A comma closes a zoned reading too, where the zone follows the fraction.
        let utc = DataType::datetime64(TimeUnit::Millisecond, Timezone::UTC).unwrap();
        assert_eq!(
            utc.scalar(Scalar::from("2026-08-14T00:05:01,148Z"))
                .unwrap(),
            utc.scalar(Scalar::from("2026-08-14T00:05:01.148Z"))
                .unwrap(),
        );

        // A comma still needs its digits, and a grouped fraction reads at the
        // width the grouping spells.
        assert!(naive.scalar(Scalar::from("2026-08-14T00:05:01,")).is_err());
        let micros = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap();
        assert_eq!(
            micros
                .scalar(Scalar::from("2026-08-14 00:05:01,147_250"))
                .unwrap(),
            micros
                .scalar(Scalar::from("2026-08-14 00:05:01.147250"))
                .unwrap(),
        );
    }
}

mod fields {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Date32Array, Date64Array};
    use yggdryl::{ArrowCastOptions, DataType, DataTypeId, Scalar, Serie, TimeUnit, Timezone};
    use yggdryl::{
        DateField, DateTimeField, DateTimeType, DateType, DurationField, DurationType,
        IntervalField, IntervalType, TimeField, TimeType,
    };

    use super::typed::assert_typed_marker;

    #[test]
    fn every_temporal_marker_covers_every_leaf_of_its_family() {
        assert_typed_marker::<DateTimeType>(
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
        );
        assert_typed_marker::<DateTimeType>(
            DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        );
        assert_typed_marker::<DateType>(DataType::date32());
        assert_typed_marker::<DateType>(DataType::date64());
        assert_typed_marker::<TimeType>(DataType::time32(TimeUnit::Second).unwrap());
        assert_typed_marker::<TimeType>(DataType::time64(TimeUnit::Microsecond).unwrap());
        assert_typed_marker::<DurationType>(DataType::duration32(TimeUnit::Millisecond).unwrap());
        assert_typed_marker::<DurationType>(DataType::duration64(TimeUnit::Millisecond).unwrap());
        assert_typed_marker::<IntervalType>(DataType::interval(TimeUnit::MonthDayNano).unwrap());
    }

    #[test]
    fn a_date_field_holds_its_own_leaf_and_the_root_redirects_to_it() {
        let field = DateField::try_new("settlement", DataType::date64(), false).unwrap();
        assert_eq!(*field.typed_dtype_ref(), DateType::Date64);
        assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Millisecond);
        assert_eq!(field.dtype(), &DataType::date64());
        assert_eq!(field.id(), DataTypeId::Date64);

        // A datatype from another family is refused by name.
        let refused = DateField::try_new(
            "settlement",
            DataType::time32(TimeUnit::Second).unwrap(),
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("date"), "{refused}");

        // The value door is the family's: text reads as the leaf the column
        // declares, at the unit that width means.
        let held = field.to_field().scalar(Scalar::from("2024-01-02")).unwrap();
        assert_eq!(held.id(), DataTypeId::Date64);
        assert_eq!(
            held.as_date64().map(|(count, ..)| count),
            Some(19_724 * 86_400_000)
        );
        let narrow = DateField::try_new("settlement", DataType::date32(), false).unwrap();
        let held = narrow
            .to_field()
            .scalar(Scalar::from("2024-01-02"))
            .unwrap();
        assert_eq!(held.as_date32().map(|(count, ..)| count), Some(19_724));
    }

    #[test]
    fn a_date_fields_array_is_the_width_its_leaf_declares() {
        // The family has no single column type: a `date32` column is a
        // `Date32Array` and a `date64` one a `Date64Array`, so the leaf says
        // which to narrow to.
        let days: ArrayRef = Arc::new(Date32Array::from(vec![Some(19_724), None]));
        let narrow = DateField::try_new("day", DataType::date32(), true).unwrap();
        let cast = Serie::from_arrow_array(
            Some(&narrow.to_field()),
            days,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        assert!(cast.as_date64().is_none());
        let cast = cast.as_date32().expect("a date32 column is a Date32 leaf");
        assert_eq!(cast.value(0), Some(19_724));
        assert_eq!(cast.value(1), None);
        assert!(cast.array().is_null(1));

        let millis: ArrayRef = Arc::new(Date64Array::from(vec![Some(19_724 * 86_400_000), None]));
        let wide = DateField::try_new("day", DataType::date64(), true).unwrap();
        let cast = Serie::from_arrow_array(
            Some(&wide.to_field()),
            millis,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        assert!(cast.as_date32().is_none());
        let cast = cast.as_date64().expect("a date64 column is a Date64 leaf");
        assert_eq!(cast.value(0), Some(19_724 * 86_400_000));
        assert_eq!(cast.value(1), None);
        assert!(cast.array().is_null(1));
    }

    #[test]
    fn a_datetime_field_reads_its_zone_and_resolution_off_its_leaf() {
        let dtype = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap();
        let field = DateTimeField::try_new("transact", dtype.clone(), false).unwrap();
        assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Microsecond);
        assert!(field.typed_dtype_ref().timezone().is_utc());
        assert_eq!(field.dtype(), &dtype);
        assert_eq!(field.id(), DataTypeId::DateTime64);
        assert!(DateTimeField::try_new("transact", DataType::date32(), false).is_err());

        // A zoned column reads a zoned instant at its own resolution.
        let held = field
            .to_field()
            .scalar(Scalar::from("2024-01-02T10:15:30Z"))
            .unwrap();
        assert_eq!(held.id(), DataTypeId::DateTime64);
        assert_eq!(
            held.as_datetime64().map(|(count, unit, _)| (count, unit)),
            Some((1_704_190_530_000_000, TimeUnit::Microsecond))
        );
        assert_eq!(held.temporal_timezone(), Some(Timezone::UTC));
    }

    #[test]
    fn a_time_field_holds_the_width_its_resolution_fits_in() {
        let field =
            TimeField::try_new("open", DataType::time32(TimeUnit::Second).unwrap(), false).unwrap();
        assert_eq!(*field.typed_dtype_ref(), TimeType::Time32(TimeUnit::Second));
        assert_eq!(field.typed_dtype_ref().bit_width(), 32);
        assert_eq!(field.dtype(), &DataType::time32(TimeUnit::Second).unwrap());
        assert_eq!(field.id(), DataTypeId::Time32);
        assert!(
            TimeField::try_new(
                "open",
                DataType::duration32(TimeUnit::Second).unwrap(),
                false
            )
            .is_err()
        );

        let held = field.to_field().scalar(Scalar::from("09:30:00")).unwrap();
        assert_eq!(held.id(), DataTypeId::Time32);
        assert_eq!(held.as_time32().map(|(count, ..)| count), Some(34_200));
    }

    #[test]
    fn a_duration_field_holds_a_fixed_length_unit_at_its_width() {
        let field = DurationField::try_new(
            "elapsed",
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            *field.typed_dtype_ref(),
            DurationType::Duration64(TimeUnit::Millisecond)
        );
        assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Millisecond);
        assert_eq!(
            field.dtype(),
            &DataType::duration64(TimeUnit::Millisecond).unwrap()
        );
        assert_eq!(field.id(), DataTypeId::Duration64);
        assert!(
            DurationField::try_new(
                "elapsed",
                DataType::time32(TimeUnit::Second).unwrap(),
                false
            )
            .is_err()
        );

        let held = field.to_field().scalar(Scalar::from("PT90S")).unwrap();
        assert_eq!(held.id(), DataTypeId::Duration64);
        assert_eq!(held.as_duration64().map(|(count, ..)| count), Some(90_000));
    }

    #[test]
    fn an_interval_field_holds_its_layout() {
        let field = IntervalField::try_new(
            "tenor",
            DataType::interval(TimeUnit::MonthDayNano).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            *field.typed_dtype_ref(),
            IntervalType::Interval(TimeUnit::MonthDayNano)
        );
        assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::MonthDayNano);
        assert_eq!(
            field.dtype(),
            &DataType::interval(TimeUnit::MonthDayNano).unwrap()
        );
        assert_eq!(field.id(), DataTypeId::Interval);
        assert!(
            IntervalField::try_new(
                "tenor",
                DataType::duration64(TimeUnit::Millisecond).unwrap(),
                false
            )
            .is_err()
        );

        // A value already in the column's layout comes back untouched.
        let span = Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap();
        let held = field.to_field().scalar(span.clone()).unwrap();
        assert_eq!(held, span);
        assert_eq!(held.id(), DataTypeId::Interval);
    }
}

mod scalars {
    use yggdryl::{DataType, DataTypeId, FamilyValue, Scalar, Temporal, TimeUnit, Timezone, Value};
    use yggdryl::{Date32, Date64, DateTime64, Duration32, Duration64, Interval, Time32, Time64};

    #[test]
    fn constructors_reject_illegal_width_unit_combinations() {
        assert!(Scalar::time32(1, TimeUnit::Microsecond, Timezone::NAIVE).is_err());
        assert!(Scalar::time64(1, TimeUnit::Millisecond, Timezone::NAIVE).is_err());
        assert!(Scalar::duration32(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration64(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration32_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::duration64_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::date32_in(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::date64_in(1, TimeUnit::Second, Timezone::NAIVE).is_err());
        // An interval refuses a component its layout has nowhere to put, and a
        // unit that is no layout.
        assert!(Scalar::interval(1, 2, 0, TimeUnit::YearMonth).is_err());
        assert!(Scalar::interval(1, 0, 0, TimeUnit::DayTime).is_err());
        assert!(Scalar::interval(0, 0, 0, TimeUnit::Second).is_err());
    }

    #[test]
    fn time_of_day_refuses_a_zone_its_datatype_would_lose() {
        for error in [
            Scalar::time32(1, TimeUnit::Second, Timezone::UTC).unwrap_err(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::UTC).unwrap_err(),
        ] {
            let message = error.to_string();
            assert!(message.contains("timezone"), "{message}");
            assert!(message.contains("no timezone"), "{message}");
        }
    }

    #[test]
    fn every_temporal_carries_a_zone() {
        let values = [
            Scalar::date32(1),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
            Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
        ];
        assert!(values.iter().all(Scalar::is_temporal));
    }

    #[test]
    fn family_constructors_select_exact_widths() {
        let values = [
            Scalar::from_date(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
            Scalar::from_date(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX), TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)
                .unwrap(),
        ];
        assert!(matches!(values[0], Scalar::Date32(_)));
        assert!(matches!(values[1], Scalar::Date64(_)));
        assert!(matches!(values[2], Scalar::Time32(_)));
        assert!(matches!(values[3], Scalar::Time64(_)));
        assert!(matches!(values[4], Scalar::DateTime64(_)));
        assert!(matches!(values[5], Scalar::Duration32(_)));
        assert!(matches!(values[6], Scalar::Duration64(_)));

        for count in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration32(_)
            ));
        }
        for count in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration64(_)
            ));
        }
    }

    #[test]
    fn temporal_readers_answer_every_temporal_variant() {
        let interval = Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap();
        let cases = [
            (
                Scalar::date32(1),
                "date",
                TimeUnit::Day,
                Timezone::NAIVE,
                1_i64,
            ),
            (
                Scalar::date64(86_400_000),
                "date",
                TimeUnit::Millisecond,
                Timezone::NAIVE,
                86_400_000,
            ),
            (
                Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                "time",
                TimeUnit::Second,
                Timezone::NAIVE,
                1,
            ),
            (
                Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
                "time",
                TimeUnit::Microsecond,
                Timezone::NAIVE,
                1,
            ),
            (
                Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
                "datetime",
                TimeUnit::Nanosecond,
                Timezone::UTC,
                1,
            ),
            (
                Scalar::duration32(-1, TimeUnit::Millisecond).unwrap(),
                "duration",
                TimeUnit::Millisecond,
                Timezone::NAIVE,
                -1,
            ),
            (
                Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
                "duration",
                TimeUnit::Microsecond,
                Timezone::NAIVE,
                1,
            ),
            (
                Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
                "interval",
                TimeUnit::MonthDayNano,
                Timezone::NAIVE,
                3,
            ),
        ];
        for (value, family, unit, zone, count) in &cases {
            assert!(value.is_temporal(), "{value:?}");
            assert_eq!(
                value.as_temporal().map(|held| held.family()),
                Some(*family),
                "{value:?}"
            );
            assert_eq!(value.temporal_unit(), Some(*unit), "{value:?}");
            assert_eq!(value.temporal_timezone(), Some(*zone), "{value:?}");
            assert_eq!(value.temporal_count(), Some(*count), "{value:?}");
        }
        // An interval restates no count across units; all three of its
        // components are read by matching `Scalar::Interval` directly, and the
        // constructor built exactly the value the struct does.
        assert_eq!(cases[7].0.temporal_count_at(TimeUnit::Nanosecond), None);
        assert!(matches!(cases[7].0, Scalar::Interval(held) if held == interval));
        assert_eq!(cases[0].0.as_date32().map(|(count, ..)| count), Some(1));
        assert_eq!(
            cases[1].0.as_date64().map(|(count, ..)| count),
            Some(86_400_000)
        );

        let number = Scalar::from(1);
        assert!(!number.is_temporal());
        assert_eq!(number.as_temporal().map(|held| held.family()), None);
        assert_eq!(number.temporal_unit(), None);
        assert_eq!(number.temporal_timezone(), None);
        assert_eq!(number.temporal_count(), None);
        assert_eq!(number.temporal_count_at(TimeUnit::Second), None);
    }

    #[test]
    fn a_temporal_leaf_is_its_own_family() {
        // A value's datatype is its family's, with the parameters the value
        // carries.
        let value = Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let leaf = <DateTime64 as Value>::from_scalar(&value).copied().unwrap();
        assert_eq!(leaf.dtype().unwrap().id(), DataTypeId::DateTime64);
        assert_eq!(
            leaf.dtype().unwrap(),
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap()
        );
        assert_eq!(<DateTime64 as Value>::into_scalar(leaf), value);
        assert!(<Date32 as Value>::from_scalar(&value).is_none());

        let day = <Date32 as Value>::from_scalar(&Scalar::date32(1))
            .copied()
            .unwrap();
        assert_eq!(day.dtype().unwrap(), DataType::date32());

        let interval = Interval::new(1, 0, 0, TimeUnit::YearMonth).unwrap();
        let held = <Interval as Value>::into_scalar(interval);
        assert_eq!(<Interval as Value>::from_scalar(&held), Some(&interval));
        assert_eq!(interval.dtype().unwrap().id(), DataTypeId::Interval);
        assert_eq!(
            interval.dtype().unwrap(),
            DataType::interval(TimeUnit::YearMonth).unwrap()
        );
    }

    #[test]
    fn family_constructors_reject_invalid_parts() {
        assert!(Scalar::from_date(1, TimeUnit::Second, Timezone::NAIVE).is_err());
        assert!(Scalar::from_date(i64::MAX, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::from_datetime(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::Second, Timezone::UTC).is_err());
    }

    #[test]
    fn the_temporal_family_stands_for_every_leaf() {
        crate::scalar::assert_family_round_trip(
            vec![
                crate::family_leaf!(
                    Temporal::Date32,
                    Date32::new(1, TimeUnit::Day, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Date64,
                    Date64::new(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Time32,
                    Time32::new(1, TimeUnit::Second, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Time64,
                    Time64::new(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::DateTime64,
                    DateTime64::new(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Duration32,
                    Duration32::new(-1, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Duration64,
                    Duration64::new(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap()
                ),
                crate::family_leaf!(
                    Temporal::Interval,
                    Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap()
                ),
            ],
            yggdryl::DataTypeKind::Temporal,
            &Scalar::from(1_i64),
        );
        // The family answers the leaf's datatype with the parameters the leaf
        // carries, as `Scalar::dtype` does for the same value.
        let value = Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        assert_eq!(
            Temporal::from_scalar(&value).unwrap().dtype().unwrap(),
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap()
        );
    }
}
