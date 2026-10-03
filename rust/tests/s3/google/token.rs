//! `rust/src/s3/google/token.rs`: the token answer no caller can name.
//!
//! Every exchange and the metadata server end in one reader of
//! `{"access_token", "expires_in"}`. A token with no stated lifetime is never
//! obtained again, so the lifetime is what the refresh policy rests on, and
//! Google states it as a number.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::internals::s3_google_token::read_token;

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

#[test]
fn a_lifetime_google_states_as_a_number_lapses_the_token() {
    // The shape the token endpoint and the metadata server both answer.
    let body = br#"{"access_token":"ya29.a0","expires_in":3599,"token_type":"Bearer"}"#;
    let (token, lapses) = read_token(body, now()).expect("a token");
    assert_eq!(token, "ya29.a0");
    assert_eq!(lapses, Some(now() + Duration::from_secs(3599)));
}

#[test]
fn a_lifetime_stated_as_text_lapses_the_token_the_same_way() {
    for lifetime in [r#""3599""#, r#"" 3599 ""#, r#""+3599""#] {
        let body = format!(r#"{{"access_token":"ya29.a0","expires_in":{lifetime}}}"#);
        let (_, lapses) = read_token(body.as_bytes(), now()).expect(&body);
        assert_eq!(
            lapses,
            Some(now() + Duration::from_secs(3599)),
            "{lifetime}"
        );
    }
}

#[test]
fn a_lifetime_no_reading_accepts_states_none_rather_than_a_guess() {
    for lifetime in ["-1", "1.5", r#""soon""#, r#""""#, "true", "null", "[3599]"] {
        let body = format!(r#"{{"access_token":"t","expires_in":{lifetime}}}"#);
        let (_, lapses) = read_token(body.as_bytes(), now()).expect(&body);
        assert_eq!(lapses, None, "{lifetime}");
    }
    let (_, lapses) = read_token(br#"{"access_token":"t"}"#, now()).expect("a token");
    assert_eq!(lapses, None);
}

#[test]
fn a_lifetime_no_clock_can_hold_lapses_nothing_rather_than_panicking() {
    for lifetime in ["18446744073709551615", r#""18446744073709551615""#] {
        let body = format!(r#"{{"access_token":"t","expires_in":{lifetime}}}"#);
        let (_, lapses) = read_token(body.as_bytes(), now()).expect(&body);
        assert_eq!(lapses, None, "{lifetime}");
    }
}

#[test]
fn an_answer_without_a_token_is_refused_by_name() {
    for body in [&br#"{"expires_in":3599}"#[..], br#"{"access_token":3599}"#] {
        let error = read_token(body, now()).expect_err("a refusal");
        assert!(error.to_string().contains("access_token"), "{error}");
    }
    for body in [&b"not json"[..], b""] {
        assert!(read_token(body, now()).is_err());
    }
}
