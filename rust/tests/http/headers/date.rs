//! `rust/src/http/headers/date.rs`: the three HTTP-date forms read and the
//! one written, every calendar refusal first.

use yggdryl::Error;
use yggdryl::http::{parse_http_date, render_http_date};

/// `Sun, 06 Nov 1994 08:49:37 GMT`, the instant every example of RFC 9110
/// spells, in nanoseconds.
const RFC_EXAMPLE_NS: i64 = 784_111_777 * 1_000_000_000;

fn refusal(text: &str) -> (usize, String) {
    match parse_http_date(text) {
        Err(Error::Parse {
            target: "http date",
            position,
            reason,
        }) => (position, reason.to_string()),
        other => panic!("{text:?} should be refused as an http date, got {other:?}"),
    }
}

#[test]
fn a_text_that_is_no_date_is_refused_naming_the_position() {
    let (position, reason) = refusal("yesterday");
    assert_eq!(position, 0);
    assert!(reason.contains("expected a day name"), "{reason}");

    assert_eq!(refusal("").0, 0);
    assert!(
        refusal("Sun, 06 Nov 1994 08:49:37 UTC")
            .1
            .contains("expected GMT")
    );
    assert!(
        refusal("Sun, 06 Nov 1994 08:49:37")
            .1
            .contains("expected ' '")
    );
    // A day name that is not one of the seven.
    assert!(
        refusal("Sonntag, 06 Nov 1994 08:49:37 GMT")
            .1
            .contains("day name")
    );
    // A month that is not one of the twelve.
    assert!(
        refusal("Sun, 06 Nvo 1994 08:49:37 GMT")
            .1
            .contains("month name")
    );
}

#[test]
fn a_calendar_value_out_of_range_is_refused() {
    assert!(
        refusal("Sun, 31 Apr 1994 08:49:37 GMT")
            .1
            .contains("a day the month has")
    );
    assert!(
        refusal("Tue, 29 Feb 1994 08:49:37 GMT")
            .1
            .contains("a day the month has")
    );
    assert!(
        refusal("Sun, 00 Nov 1994 08:49:37 GMT")
            .1
            .contains("a day the month has")
    );
    assert!(
        refusal("Sun, 06 Nov 1994 24:00:00 GMT")
            .1
            .contains("time of day")
    );
    assert!(
        refusal("Sun, 06 Nov 1994 08:60:37 GMT")
            .1
            .contains("time of day")
    );
    assert!(
        refusal("Sun, 06 Nov 1994 08:49:61 GMT")
            .1
            .contains("time of day")
    );
}

#[test]
fn trailing_text_and_a_form_mixed_with_another_are_refused() {
    assert!(
        refusal("Sun, 06 Nov 1994 08:49:37 GMT extra")
            .1
            .contains("end of the date")
    );
    // A two-digit year in the IMF-fixdate form.
    assert!(refusal("Sun, 06 Nov 94 08:49:37 GMT").1.contains("digit"));
    // An RFC 850 date under a short day name.
    assert!(
        refusal("Sun, 06-Nov-94 08:49:37 GMT")
            .1
            .contains("expected ' '")
    );
    // asctime with a padded two-digit day.
    assert!(refusal("Sun Nov  16 08:49:37 1994").1.contains("expected"));
}

#[test]
fn imf_fixdate_reads_to_the_epoch_instant() {
    assert_eq!(
        parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap(),
        RFC_EXAMPLE_NS
    );
    assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT").unwrap(), 0);
    assert_eq!(
        parse_http_date("Wed, 31 Dec 1969 23:59:59 GMT").unwrap(),
        -1_000_000_000
    );
    // A leap day of a leap year, and a leap second.
    assert_eq!(
        parse_http_date("Tue, 29 Feb 2000 00:00:00 GMT").unwrap(),
        951_782_400 * 1_000_000_000
    );
    assert_eq!(
        parse_http_date("Thu, 01 Jan 1970 00:00:60 GMT").unwrap(),
        60 * 1_000_000_000
    );
}

#[test]
fn the_day_name_is_read_but_its_agreement_is_not_checked() {
    assert_eq!(
        parse_http_date("Mon, 06 Nov 1994 08:49:37 GMT").unwrap(),
        RFC_EXAMPLE_NS
    );
}

#[test]
fn rfc850_reads_the_same_instant_with_a_two_digit_year() {
    assert_eq!(
        parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").unwrap(),
        RFC_EXAMPLE_NS
    );
    // `70`-`99` are the twentieth century and `00`-`69` the twenty-first.
    assert_eq!(
        parse_http_date("Thursday, 01-Jan-70 00:00:00 GMT").unwrap(),
        0
    );
    assert_eq!(
        parse_http_date("Saturday, 01-Jan-00 00:00:00 GMT").unwrap(),
        946_684_800 * 1_000_000_000
    );
    assert_eq!(
        parse_http_date("Wednesday, 01-Jan-69 00:00:00 GMT").unwrap(),
        3_124_224_000 * 1_000_000_000
    );
}

#[test]
fn asctime_reads_the_same_instant_with_a_padded_day() {
    assert_eq!(
        parse_http_date("Sun Nov  6 08:49:37 1994").unwrap(),
        RFC_EXAMPLE_NS
    );
    assert_eq!(
        parse_http_date("Wed Nov 16 08:49:37 1994").unwrap(),
        RFC_EXAMPLE_NS + 10 * 86_400 * 1_000_000_000
    );
}

#[test]
fn rendering_writes_imf_fixdate_and_reads_back() {
    assert_eq!(
        render_http_date(RFC_EXAMPLE_NS),
        "Sun, 06 Nov 1994 08:49:37 GMT"
    );
    assert_eq!(render_http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
    // A fraction of a second is dropped, toward the past.
    assert_eq!(
        render_http_date(RFC_EXAMPLE_NS + 999_999_999),
        "Sun, 06 Nov 1994 08:49:37 GMT"
    );
    assert_eq!(render_http_date(-1), "Wed, 31 Dec 1969 23:59:59 GMT");
    for ns in [
        0,
        RFC_EXAMPLE_NS,
        -86_400_000_000_000,
        4_102_444_800_000_000_000,
    ] {
        assert_eq!(parse_http_date(&render_http_date(ns)).unwrap(), ns);
    }
}
