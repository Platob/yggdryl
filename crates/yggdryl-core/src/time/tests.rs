//! Tests for the core time module: civil algorithms, the four value types and the
//! self-contained DST engine.

use super::*;

#[test]
fn civil_algorithms_round_trip() {
    for &(y, m, d) in &[
        (1970, 1, 1),
        (2000, 2, 29),
        (2024, 2, 29),
        (1, 1, 1),
        (1999, 12, 31),
        (2262, 4, 11),
        (-1, 12, 31),
    ] {
        let days = days_from_civil(y, m, d);
        assert_eq!(civil_from_days(days), (y, m, d), "{y}-{m}-{d}");
    }
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(1970, 1, 2), 1);
    assert_eq!(days_from_civil(1969, 12, 31), -1);
    assert!(is_leap_year(2000) && is_leap_year(2024) && !is_leap_year(1900) && !is_leap_year(2023));
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2023, 2), 28);
}

#[test]
fn time_unit_parsing_and_nanos() {
    assert_eq!(TimeUnit::from_str("ms").unwrap(), TimeUnit::Millisecond);
    assert_eq!(
        TimeUnit::from_str("microseconds").unwrap(),
        TimeUnit::Microsecond
    );
    assert_eq!(TimeUnit::Second.nanos(), 1_000_000_000);
    assert_eq!(TimeUnit::Nanosecond.per_second(), 1_000_000_000);
    assert!(TimeUnit::Second < TimeUnit::Nanosecond);
    assert!(TimeUnit::from_str("weeks").is_err());
}

#[test]
fn date_parse_render_and_validate() {
    let d = Date::from_str("2024-02-29").unwrap();
    assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
    assert_eq!(d.to_str(), "2024-02-29");
    // 2024-02-29 was a Thursday (weekday 4).
    assert_eq!(d.weekday(), 4);
    assert_eq!(d.add_days(1).to_str(), "2024-03-01");
    assert!(Date::from_ymd(2023, 2, 29).is_err());
    assert!(Date::from_str("2024-13-01").is_err());
    assert!(Date::from_str("not-a-date").is_err());
    // mapping + bytes round-trips.
    assert_eq!(Date::from_mapping(&d.to_mapping()).unwrap(), d);
    assert_eq!(Date::from_bytes(&d.to_bytes()).unwrap(), d);
    // negative (BCE) years.
    let bce = Date::from_str("-0044-03-15").unwrap();
    assert_eq!(bce.year(), -44);
    assert_eq!(bce.to_str(), "-0044-03-15");
}

#[test]
fn time_parse_render_fractions() {
    let t = Time::from_str("13:45:30.250").unwrap();
    assert_eq!(
        (t.hour(), t.minute(), t.second(), t.nanosecond()),
        (13, 45, 30, 250_000_000)
    );
    assert_eq!(t.to_str(), "13:45:30.250");
    assert_eq!(Time::from_str("09:05").unwrap().to_str(), "09:05:00");
    assert_eq!(
        Time::from_str("00:00:00.000000001").unwrap().nanosecond(),
        1
    );
    assert!(Time::from_str("24:00:00").is_err());
    assert!(Time::from_str("12:60:00").is_err());
    let t2 = Time::from_hms(1, 2, 3).unwrap();
    assert_eq!(Time::from_mapping(&t2.to_mapping()).unwrap(), t2);
    assert!(Time::from_str("00:00:00").unwrap() < Time::from_str("00:00:01").unwrap());
}

#[test]
fn duration_parse_render_arith() {
    let d = Duration::from_str("1h30m").unwrap();
    assert_eq!(d.as_seconds(), 5_400);
    assert_eq!(d.to_str(), "1h30m");
    assert_eq!(Duration::from_str("90").unwrap().as_seconds(), 90); // bare = seconds
    assert_eq!(Duration::from_str("1.5").unwrap().as_nanos(), 1_500_000_000);
    assert_eq!(Duration::from_str("1s500ms").unwrap().to_str(), "1s500ms");
    assert_eq!(Duration::from_str("-2d").unwrap().to_str(), "-2d");
    assert!(Duration::from_secs(-5).is_negative());
    assert_eq!(
        Duration::from_secs(5)
            .add(&Duration::from_secs(3))
            .as_seconds(),
        8
    );
    assert_eq!(Duration::from_str("250us").unwrap().to_str(), "250us");
    assert_eq!(
        Duration::from_unit(3, TimeUnit::Millisecond).as_nanos(),
        3_000_000
    );
    assert_eq!(Duration::default().to_str(), "0s");
    // round-trip a mix.
    let mix = Duration::from_str("2d3h4m5s6ms").unwrap();
    assert_eq!(Duration::from_str(&mix.to_str()).unwrap(), mix);
    assert!(Duration::from_str("5x").is_err());
}

#[test]
fn timezone_fixed_and_named() {
    assert_eq!(Timezone::from_str("UTC").unwrap(), Timezone::Utc);
    assert_eq!(Timezone::from_str("Z").unwrap(), Timezone::Utc);
    assert_eq!(Timezone::from_str("+00:00").unwrap(), Timezone::Utc);
    assert_eq!(
        Timezone::from_str("+05:30").unwrap(),
        Timezone::Fixed(19_800)
    );
    assert_eq!(
        Timezone::from_str("-08:00").unwrap().offset_seconds(0),
        -28_800
    );
    assert_eq!(
        Timezone::from_str("+0530").unwrap(),
        Timezone::Fixed(19_800)
    );
    assert_eq!(Timezone::from_str("+05:30").unwrap().to_str(), "+05:30");
    assert!(Timezone::from_str("Mars/Olympus").is_err());
    // no-DST named zones.
    assert_eq!(
        Timezone::from_str("Asia/Tokyo").unwrap().offset_seconds(0),
        9 * 3600
    );
    assert_eq!(
        Timezone::from_str("Asia/Kolkata")
            .unwrap()
            .offset_seconds(0),
        5 * 3600 + 1800
    );
    assert_eq!(
        Timezone::from_str("Asia/Kathmandu")
            .unwrap()
            .offset_seconds(0),
        5 * 3600 + 2700
    );
}

/// Helper: UTC epoch seconds for a civil UTC datetime.
fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h as i64 * 3600 + mi as i64 * 60
}

#[test]
fn timezone_dst_northern_hemisphere() {
    let ny = Timezone::from_str("America/New_York").unwrap();
    // January = EST (UTC-5), July = EDT (UTC-4).
    assert_eq!(ny.offset_seconds(utc(2024, 1, 15, 12, 0)), -5 * 3600);
    assert_eq!(ny.offset_seconds(utc(2024, 7, 15, 12, 0)), -4 * 3600);
    // The 2024 spring-forward is 2024-03-10 07:00 UTC (02:00 EST -> 03:00 EDT).
    assert_eq!(ny.offset_seconds(utc(2024, 3, 10, 6, 59)), -5 * 3600);
    assert_eq!(ny.offset_seconds(utc(2024, 3, 10, 7, 0)), -4 * 3600);
    // EU: Paris is +1 in winter, +2 in summer; switch at 01:00 UTC.
    let paris = Timezone::from_str("Europe/Paris").unwrap();
    assert_eq!(paris.offset_seconds(utc(2024, 1, 15, 12, 0)), 3600);
    assert_eq!(paris.offset_seconds(utc(2024, 7, 15, 12, 0)), 7200);
    assert_eq!(paris.offset_seconds(utc(2024, 3, 31, 0, 59)), 3600);
    assert_eq!(paris.offset_seconds(utc(2024, 3, 31, 1, 0)), 7200);
}

#[test]
fn timezone_dst_southern_hemisphere() {
    let sydney = Timezone::from_str("Australia/Sydney").unwrap();
    // Southern hemisphere: DST in the local summer (Jan = +11), standard in winter (Jul = +10).
    assert_eq!(sydney.offset_seconds(utc(2024, 1, 15, 12, 0)), 11 * 3600);
    assert_eq!(sydney.offset_seconds(utc(2024, 7, 15, 12, 0)), 10 * 3600);
    // Auckland: +13 (NZDT) in January, +12 (NZST) in July.
    let auckland = Timezone::from_str("Pacific/Auckland").unwrap();
    assert_eq!(auckland.offset_seconds(utc(2024, 1, 15, 12, 0)), 13 * 3600);
    assert_eq!(auckland.offset_seconds(utc(2024, 7, 15, 12, 0)), 12 * 3600);
}

#[test]
fn datetime_instant_and_conversion() {
    let utc_dt = DateTime::from_str("2024-07-01T12:00:00Z").unwrap();
    assert_eq!(utc_dt.epoch_seconds(), 1_719_835_200);
    assert_eq!(utc_dt.to_str(), "2024-07-01T12:00:00Z");
    // Same instant, different display zones (DST-aware).
    let ny = utc_dt.to_timezone(Timezone::from_str("America/New_York").unwrap());
    assert_eq!((ny.hour(), ny.minute()), (8, 0));
    assert_eq!(ny.to_str(), "2024-07-01T08:00:00-04:00");
    let tokyo = utc_dt.to_timezone(Timezone::from_str("Asia/Tokyo").unwrap());
    assert_eq!(tokyo.hour(), 21);
    // The instant never changes across display conversions.
    assert_eq!(ny.epoch_seconds(), utc_dt.epoch_seconds());
    assert_eq!(tokyo.epoch_seconds(), utc_dt.epoch_seconds());
}

#[test]
fn datetime_localize_and_parse_offset() {
    // A wall-clock time in New York resolves to the right UTC instant (EDT in July).
    let dt = DateTime::from_ymd_hms(
        2024,
        7,
        1,
        8,
        0,
        0,
        0,
        Some(Timezone::from_str("America/New_York").unwrap()),
    )
    .unwrap();
    assert_eq!(dt.epoch_seconds(), 1_719_835_200); // == 12:00 UTC
                                                   // Parsing an explicit offset.
    let off = DateTime::from_str("2024-07-01T08:00:00-04:00").unwrap();
    assert_eq!(off.epoch_seconds(), 1_719_835_200);
    // Naive datetimes have no offset suffix and are treated as UTC for the instant.
    let naive = DateTime::from_str("2024-07-01T12:00:00").unwrap();
    assert_eq!(naive.timezone(), None);
    assert_eq!(naive.epoch_seconds(), 1_719_835_200);
    assert_eq!(naive.to_str(), "2024-07-01T12:00:00");
    // mapping + bytes round-trips.
    assert_eq!(
        DateTime::from_bytes(&utc_now_like().to_bytes())
            .unwrap()
            .epoch_seconds(),
        utc_now_like().epoch_seconds()
    );
}

fn utc_now_like() -> DateTime {
    DateTime::from_str("2024-03-10T07:00:00Z").unwrap()
}

#[cfg(feature = "serde")]
#[test]
fn temporal_serde_round_trips() {
    let date = Date::from_str("2024-02-29").unwrap();
    let time = Time::from_str("13:45:30.250").unwrap();
    let dt = DateTime::from_str("2024-07-01T08:00:00-04:00").unwrap();
    let dur = Duration::from_str("1h30m").unwrap();
    let tz = Timezone::from_str("America/New_York").unwrap();
    assert_eq!(
        serde_json::from_str::<Date>(&serde_json::to_string(&date).unwrap()).unwrap(),
        date
    );
    assert_eq!(
        serde_json::from_str::<Time>(&serde_json::to_string(&time).unwrap()).unwrap(),
        time
    );
    assert_eq!(
        serde_json::from_str::<DateTime>(&serde_json::to_string(&dt).unwrap()).unwrap(),
        dt
    );
    assert_eq!(
        serde_json::from_str::<Duration>(&serde_json::to_string(&dur).unwrap()).unwrap(),
        dur
    );
    // Timezone serialises as its canonical string.
    assert_eq!(serde_json::to_string(&tz).unwrap(), "\"America/New_York\"");
    assert_eq!(
        serde_json::from_str::<Timezone>("\"America/New_York\"").unwrap(),
        tz
    );
}
