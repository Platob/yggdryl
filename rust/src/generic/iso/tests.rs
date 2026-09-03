use super::*;

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
    assert!(parse_time("24:00:00").is_err());
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
    let (count, unit, zone) = parse_timestamp("1970-01-01T05:30:00+05:30[Asia/Kolkata]").unwrap();
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
}
