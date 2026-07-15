//! The `io::fixed::temporal` family: [`TimeUnit`], the [`Tz`] timezone (DST-aware via the IANA
//! database), and the value types [`Date32`]/[`Date64`], [`Time32`]/[`Time64`],
//! [`Ts32`]/[`Ts64`]/[`Ts96`], and [`Duration32`]/[`Duration64`] — their
//! calendar math, unit / timezone conversions, ISO-8601 `Display`/`FromStr`, and byte codecs, with
//! the edge cases each has to get right (leap years, DST transitions, negative dates, overflow).

use std::str::FromStr;

use yggdryl_core::io::fixed::temporal::{
    Date32, Date64, Duration32, Duration64, Temporal, TemporalError, Time32, Time64, TimeUnit,
    Ts32, Ts64, Ts96, Tz,
};

// -------------------------------------------------------------------------------------
// TimeUnit
// -------------------------------------------------------------------------------------

#[test]
fn time_unit_conversions_and_parsing() {
    assert_eq!(TimeUnit::Second.nanos(), Some(1_000_000_000));
    assert_eq!(TimeUnit::Year.nanos(), None); // calendar unit, no fixed length
    assert!(TimeUnit::Day.is_fixed() && TimeUnit::Month.is_calendar());
    assert_eq!(
        TimeUnit::convert(1500, TimeUnit::Millisecond, TimeUnit::Second),
        Some(1)
    ); // truncates
    assert_eq!(
        TimeUnit::convert(2, TimeUnit::Second, TimeUnit::Millisecond),
        Some(2000)
    );
    assert_eq!(TimeUnit::convert(1, TimeUnit::Second, TimeUnit::Year), None); // calendar
    for text in [
        "ns",
        "nanosecond",
        "us",
        "µs",
        "ms",
        "s",
        "sec",
        "min",
        "minutes",
        "h",
        "d",
        "w",
        "mo",
        "y",
        "years",
    ] {
        assert!(TimeUnit::parse(text).is_some(), "{text}");
    }
    assert_eq!(TimeUnit::Nanosecond.to_string(), "ns");
    assert!(TimeUnit::parse("fortnight").is_none());
}

// -------------------------------------------------------------------------------------
// Date
// -------------------------------------------------------------------------------------

#[test]
fn date_epoch_leap_years_and_round_trip() {
    assert_eq!(Date32::from_days(0).to_ymd(), (1970, 1, 1)); // the epoch
    let leap = Date32::from_ymd(2024, 2, 29).unwrap();
    assert_eq!(leap.to_ymd(), (2024, 2, 29));
    assert!(leap.is_leap_year());
    // 2023 is not a leap year: Feb 29 is rejected with a guided error.
    assert!(matches!(
        Date32::from_ymd(2023, 2, 29),
        Err(TemporalError::InvalidDate { .. })
    ));
    // Every day of 2024 round-trips through days_from_civil / civil_from_days.
    let mut date = Date32::from_ymd(2024, 1, 1).unwrap();
    for _ in 0..366 {
        let (y, m, d) = date.to_ymd();
        assert_eq!(Date32::from_ymd(y, m, d).unwrap(), date);
        date = Date32::from_days(date.days() + 1);
    }
    // Weekday: 1970-01-01 was a Thursday (0=Sun -> 4).
    assert_eq!(Date32::from_days(0).weekday(), 4);
    // Negative (pre-epoch) date.
    assert_eq!(Date32::from_ymd(1900, 1, 1).unwrap().to_ymd(), (1900, 1, 1));
}

#[test]
fn date_display_parse_and_width_conversion() {
    let d = Date32::from_ymd(2024, 2, 29).unwrap();
    assert_eq!(d.to_string(), "2024-02-29");
    assert_eq!(Date32::from_str("2024-02-29").unwrap(), d);
    assert_eq!(Date32::from_str("-0044-03-15").unwrap().year(), -44); // ides of March, 44 BC
    assert!(Date32::from_str("2024-13-01").is_err());
    // Date32 <-> Date64.
    assert_eq!(d.to_date64().to_ymd(), (2024, 2, 29));
    assert_eq!(
        Date64::from_ymd(2024, 2, 29).unwrap().to_date32().unwrap(),
        d
    );
    // Codec round-trip.
    assert_eq!(Date32::deserialize_bytes(&d.serialize_bytes()).unwrap(), d);
    assert_eq!(<Date32 as Temporal>::time_unit(&d), TimeUnit::Day);
}

// -------------------------------------------------------------------------------------
// Time
// -------------------------------------------------------------------------------------

#[test]
fn time_components_units_and_parse() {
    let t = Time32::from_hms(13, 45, 30).unwrap();
    assert_eq!(t.to_hms(), (13, 45, 30, 0));
    assert_eq!(t.to_string(), "13:45:30");
    assert_eq!(t.nanos_of_day(), (13 * 3600 + 45 * 60 + 30) * 1_000_000_000);
    // Second -> millisecond widening, and Time32 -> Time64.
    assert_eq!(
        t.to_unit(TimeUnit::Millisecond).unwrap().value(),
        (13 * 3600 + 45 * 60 + 30) * 1000
    );
    let nanos = t.to_time64(TimeUnit::Nanosecond).unwrap();
    assert_eq!(nanos.to_hms(), (13, 45, 30, 0));
    // Nanosecond time with a fraction.
    let ns = Time64::from_hms_nano(1, 2, 3, 456_000_000).unwrap();
    assert_eq!(ns.to_string(), "01:02:03.456000000");
    assert_eq!(
        Time64::from_str("01:02:03.456").unwrap().to_hms(),
        (1, 2, 3, 456_000_000)
    );
    // Out of range / invalid.
    assert!(matches!(
        Time32::from_hms(24, 0, 0),
        Err(TemporalError::InvalidTime { .. })
    ));
    assert!(matches!(
        Time32::new(1, TimeUnit::Nanosecond),
        Err(TemporalError::UnsupportedUnit { .. })
    ));
    // Codec.
    assert_eq!(Time32::deserialize_bytes(&t.serialize_bytes()).unwrap(), t);
}

// -------------------------------------------------------------------------------------
// Timezone (DST-aware, via the IANA database)
// -------------------------------------------------------------------------------------

#[test]
fn timezone_offsets_and_dst() {
    assert_eq!(Tz::UTC.offset_seconds_at(0), 0);
    assert_eq!(
        Tz::fixed_hours_minutes(-5, 0).offset_seconds_at(0),
        -5 * 3600
    );
    assert!(Tz::NAIVE.is_naive() && Tz::naive().offset_seconds_at(0) == 0);

    // Europe/Paris: +1h in winter (CET), +2h in summer (CEST) — a real DST transition.
    let paris = Tz::europe_paris();
    let winter = Ts64::from_datetime(2024, 1, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    let summer = Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(paris.offset_seconds_at(winter.epoch_seconds()), 3600); // CET
    assert_eq!(paris.offset_seconds_at(summer.epoch_seconds()), 7200); // CEST

    // Parsing: IANA name, offset, UTC/Z, naive, Windows-ish prefix.
    assert!(Tz::parse("Europe/Paris").unwrap().is_iana());
    assert_eq!(Tz::parse("+02:00").unwrap().offset_seconds_at(0), 7200);
    assert_eq!(
        Tz::parse("UTC-05:00").unwrap().offset_seconds_at(0),
        -5 * 3600
    );
    assert!(Tz::parse("Z").unwrap().is_utc());
    assert!(Tz::parse("").unwrap().is_naive());
    assert!(Tz::parse("Not/AZone").is_none());
}

// -------------------------------------------------------------------------------------
// Timestamp — the instant, zone-aware
// -------------------------------------------------------------------------------------

#[test]
fn timestamp_wall_clock_moves_with_the_zone() {
    // The SAME instant reads differently in different zones.
    let instant = Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(instant.to_datetime(), (2024, 7, 15, 12, 0, 0, 0));
    let paris = instant.with_timezone(Tz::europe_paris());
    assert_eq!(paris.to_datetime(), (2024, 7, 15, 14, 0, 0, 0)); // +2h in summer
    assert_eq!(paris.epoch_value(), instant.epoch_value()); // same stored instant
    let ny = instant.with_timezone(Tz::america_new_york());
    assert_eq!(ny.to_datetime(), (2024, 7, 15, 8, 0, 0, 0)); // -4h (EDT)

    // Winter: Paris is +1h.
    let winter = Ts64::from_datetime(2024, 1, 1, 0, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(
        winter.with_timezone(Tz::europe_paris()).to_datetime(),
        (2024, 1, 1, 1, 0, 0, 0)
    );
}

#[test]
fn timestamp_construct_convert_and_extract() {
    // from_datetime in a fixed zone resolves to the right UTC instant.
    let local = Ts64::from_datetime(
        2024,
        7,
        15,
        14,
        0,
        0,
        0,
        TimeUnit::Second,
        Tz::fixed_hours_minutes(2, 0),
    )
    .unwrap();
    assert_eq!(
        local.epoch_seconds(),
        Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC)
            .unwrap()
            .epoch_seconds()
    );

    // Unit conversion keeps the instant.
    let secs = Ts64::from_epoch(1_700_000_000, TimeUnit::Second, Tz::UTC).unwrap();
    let millis = secs.to_unit(TimeUnit::Millisecond).unwrap();
    assert_eq!(millis.epoch_value(), 1_700_000_000_000);
    assert_eq!(millis.epoch_seconds(), secs.epoch_seconds());

    // Extract date + time (in the value's zone).
    let ts = Ts64::from_datetime(2024, 2, 29, 13, 45, 30, 0, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(
        ts.to_date().unwrap(),
        Date32::from_ymd(2024, 2, 29).unwrap()
    );
    assert_eq!(ts.to_time().unwrap().to_hms(), (13, 45, 30, 0));
    assert_eq!(
        Date32::from_ymd(2024, 2, 29)
            .unwrap()
            .at_midnight(TimeUnit::Second, Tz::UTC)
            .unwrap()
            .to_datetime(),
        (2024, 2, 29, 0, 0, 0, 0)
    );
}

#[test]
fn timestamp_display_parse_and_widths() {
    let naive =
        Ts64::from_datetime(2024, 2, 29, 13, 45, 30, 0, TimeUnit::Second, Tz::NAIVE).unwrap();
    assert_eq!(naive.to_string(), "2024-02-29T13:45:30");
    let utc = naive.with_timezone(Tz::UTC);
    assert_eq!(utc.to_string(), "2024-02-29T13:45:30Z");
    let offset = Ts64::from_datetime(
        2024,
        2,
        29,
        13,
        45,
        30,
        0,
        TimeUnit::Second,
        Tz::fixed_hours_minutes(2, 0),
    )
    .unwrap();
    assert!(offset.to_string().ends_with("+02:00"));
    // Parse round-trips the instant.
    assert_eq!(
        Ts64::from_str("2024-02-29T13:45:30Z")
            .unwrap()
            .epoch_seconds(),
        utc.epoch_seconds()
    );
    assert_eq!(
        Ts64::from_str("2024-02-29T13:45:30").unwrap().to_datetime(),
        (2024, 2, 29, 13, 45, 30, 0)
    );
    // Width conversions + range guard.
    assert_eq!(
        naive.to_ts96().to_ts64().unwrap().epoch_value(),
        naive.epoch_value()
    );
    assert!(matches!(
        Ts32::from_epoch(i64::MAX as i128, TimeUnit::Second, Tz::UTC),
        Err(TemporalError::OutOfRange { .. })
    ));
    // Ts96 holds a nanosecond count far beyond i64's ~292-year range.
    let far = Ts96::from_datetime(5000, 1, 1, 0, 0, 0, 0, TimeUnit::Nanosecond, Tz::UTC).unwrap();
    assert_eq!(far.year(), 5000);
    // Codec round-trip (value + unit + zone).
    assert_eq!(
        Ts64::deserialize_bytes(&offset.serialize_bytes()).unwrap(),
        offset
    );
    assert_eq!(
        Ts96::deserialize_bytes(&far.serialize_bytes()).unwrap(),
        far
    );
}

// -------------------------------------------------------------------------------------
// Duration
// -------------------------------------------------------------------------------------

#[test]
fn flexible_parsing_of_common_formats() {
    // Dates: ISO, US slash, European dotted, and month names all reach the same date.
    let target = Date32::from_ymd(2024, 2, 29).unwrap();
    for text in [
        "2024-02-29",
        "2024/02/29",
        "02/29/2024",
        "29.02.2024",
        "Feb 29, 2024",
        "29 Feb 2024",
    ] {
        assert_eq!(Date32::parse_str(text).unwrap(), target, "{text}");
    }
    // Times: 24-hour and 12-hour, with/without a fraction.
    assert_eq!(
        Time32::parse_str("1:45 PM").unwrap().to_hms(),
        (13, 45, 0, 0)
    );
    assert_eq!(
        Time64::parse_str("12:00:00 AM").unwrap().to_hms(),
        (0, 0, 0, 0)
    ); // midnight
    assert_eq!(
        Time64::parse_str("13:45:30.5").unwrap().to_hms(),
        (13, 45, 30, 500_000_000)
    );

    // Timestamps: flexible datetime, a date-only (→ midnight), and a trailing zone; `unit`/`tz`
    // default and cast while parsing.
    let ts = Ts64::parse_str("2024-02-29 13:45:30", TimeUnit::Millisecond, Tz::UTC).unwrap();
    assert_eq!(ts.to_datetime(), (2024, 2, 29, 13, 45, 30, 0));
    assert!(ts.tz().is_utc() && ts.unit() == TimeUnit::Millisecond); // defaulted + cast
                                                                     // A zone in the string overrides the default.
    let zoned = Ts64::parse_str("2024-07-15T12:00:00-05:00", TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(zoned.offset_seconds(), -5 * 3600);
    // A date only lands at midnight in the default zone.
    assert_eq!(
        Ts64::parse_str("03/15/2024", TimeUnit::Second, Tz::UTC)
            .unwrap()
            .to_datetime(),
        (2024, 3, 15, 0, 0, 0, 0)
    );
    assert!(Ts64::parse_str("not a date", TimeUnit::Second, Tz::UTC).is_err());
}

#[test]
fn debug_shows_signature_and_iso_value() {
    // The Debug form prints the type signature (unit / tz) and the ISO value.
    let ts = Ts64::from_datetime(2024, 2, 29, 13, 45, 30, 0, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(format!("{ts:?}"), "ts64[s, UTC](2024-02-29T13:45:30Z)");
    let naive = ts.with_timezone(Tz::NAIVE);
    assert_eq!(format!("{naive:?}"), "ts64[s](2024-02-29T13:45:30)");
    // A nanosecond timestamp shows the full fraction.
    let nanos = ts.to_unit(TimeUnit::Nanosecond).unwrap();
    assert_eq!(
        format!("{nanos:?}"),
        "ts64[ns, UTC](2024-02-29T13:45:30.000000000Z)"
    );
    assert_eq!(
        format!("{:?}", Date32::from_ymd(2024, 2, 29).unwrap()),
        "date32(2024-02-29)"
    );
    assert_eq!(
        format!("{:?}", Duration64::milliseconds(1500)),
        "duration64[ms](1500ms)"
    );
}

#[test]
fn duration_arithmetic_units_and_parse() {
    // Add aligns to the finer unit: 1s + 500ms = 1500ms.
    let sum = Duration64::seconds(1)
        .checked_add(&Duration64::milliseconds(500))
        .unwrap();
    assert_eq!((sum.value(), sum.unit()), (1500, TimeUnit::Millisecond));
    assert_eq!(Duration64::seconds(90).to_string(), "90s");
    assert_eq!(Duration64::from_str("-1500ms").unwrap().value(), -1500);
    assert_eq!(
        Duration64::from_str("5 min").unwrap().unit(),
        TimeUnit::Minute
    );
    assert_eq!(
        Duration32::seconds(2)
            .to_unit(TimeUnit::Millisecond)
            .unwrap()
            .value(),
        2000
    );
    assert!(matches!(
        Duration32::new(1, TimeUnit::Year),
        Err(TemporalError::UnsupportedUnit { .. })
    ));
    // Ordering is by elapsed span, not raw count: 1s > 500ms even across units.
    assert!(Duration64::seconds(1) > Duration64::milliseconds(500));
    // Codec.
    let d = Duration64::milliseconds(1234);
    assert_eq!(
        Duration64::deserialize_bytes(&d.serialize_bytes()).unwrap(),
        d
    );
}

#[test]
fn temporal_cross_type_converters() {
    let date = Date32::from_ymd(2024, 2, 29).unwrap();
    let time = Time64::from_hms_nano(13, 45, 30, 0).unwrap();

    // Date <-> Timestamp — at midnight and at a wall-clock time; the date round-trips.
    let midnight = date.at_midnight(TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(midnight.to_datetime(), (2024, 2, 29, 0, 0, 0, 0));
    assert_eq!(midnight.to_date().unwrap(), date);
    let dt = date.at_time(&time, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(dt.to_datetime(), (2024, 2, 29, 13, 45, 30, 0));
    assert_eq!(dt.to_time().unwrap().to_hms(), (13, 45, 30, 0));

    // Date <-> Duration — days since the epoch, both widths.
    let span = date.to_duration();
    assert_eq!(
        (span.value(), span.unit()),
        (date.days() as i64, TimeUnit::Day)
    );
    assert_eq!(span.to_date().unwrap(), date);
    assert_eq!(
        Date64::from_ymd(2024, 2, 29)
            .unwrap()
            .to_duration()
            .to_date()
            .unwrap(),
        date
    );

    // Time <-> Duration (span since midnight) and Time -> Timestamp on the epoch date.
    assert_eq!(
        time.to_duration().to_time().unwrap().to_hms(),
        (13, 45, 30, 0)
    );
    let on_epoch = time.to_timestamp(TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(on_epoch.to_datetime(), (1970, 1, 1, 13, 45, 30, 0));

    // Timestamp <-> Duration — elapsed since the epoch round-trips the instant.
    let epoch_span = midnight.to_duration().unwrap();
    assert_eq!(
        epoch_span.to_timestamp(Tz::UTC).unwrap().epoch_value(),
        midnight.epoch_value()
    );

    // Duration widths.
    assert_eq!(
        Duration64::seconds(90).to_duration32().unwrap(),
        Duration32::seconds(90)
    );
    assert_eq!(
        Duration32::seconds(90).to_duration64(),
        Duration64::seconds(90)
    );
    assert!(Duration64::seconds(i64::from(i32::MAX) + 1)
        .to_duration32()
        .is_err());
}

#[test]
fn duration_flexible_parse_str() {
    let cases = [
        // compound unit runs — result unit is the coarsest that stays exact
        ("1h30m", 90, TimeUnit::Minute),
        ("1h30m15s", 5415, TimeUnit::Second),
        ("2d 3h", 51, TimeUnit::Hour),
        ("1 hour 30 minutes", 90, TimeUnit::Minute),
        // clock forms
        ("1:30:00", 90, TimeUnit::Minute),
        ("0:00:00.5", 500, TimeUnit::Millisecond),
        ("30:00", 30, TimeUnit::Minute),
        // ISO-8601
        ("PT1H30M", 90, TimeUnit::Minute),
        ("P1DT2H", 26, TimeUnit::Hour),
        ("P2W", 2, TimeUnit::Week),
        ("PT1.5S", 1500, TimeUnit::Millisecond),
        // single-unit + sign (a strict superset of the old FromStr)
        ("90s", 90, TimeUnit::Second),
        ("-1h30m", -90, TimeUnit::Minute),
        ("+1500ms", 1500, TimeUnit::Millisecond),
    ];
    for (text, value, unit) in cases {
        let d = Duration64::parse_str(text).unwrap();
        assert_eq!((d.value(), d.unit()), (value, unit), "{text:?}");
    }
    // Round-trips through its own Display for the single-unit forms.
    assert_eq!(Duration64::parse_str("90s").unwrap().to_string(), "90s");
    // A calendar unit has no fixed length — rejected.
    assert!(Duration64::parse_str("1mo").is_err());
    assert!(Duration64::parse_str("2 years").is_err());
    // Overflow of the narrow width is reported, not wrapped.
    assert!(matches!(
        Duration32::parse_str("100000000000ns"),
        Err(TemporalError::OutOfRange { .. })
    ));
    // Junk is a guided parse error.
    assert!(Duration64::parse_str("not a duration").is_err());
}
