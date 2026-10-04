//! `rust/src/s3/azure/auth.rs`: the token answer no caller can name.
//!
//! A hosted identity endpoint states `expires_in` as text and Entra ID and the
//! metadata service as a number; both lapse the token the same way.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::internals::s3_azure_auth::read_token;

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

#[test]
fn a_lifetime_stated_either_way_lapses_the_token() {
    for lifetime in ["3599", r#""3599""#, r#"" 3599 ""#] {
        let body = format!(r#"{{"access_token":"eyJ0","expires_in":{lifetime}}}"#);
        let (token, lapses) = read_token(body.as_bytes(), now()).expect(&body);
        assert_eq!(token, "eyJ0");
        assert_eq!(
            lapses,
            Some(now() + Duration::from_secs(3599)),
            "{lifetime}"
        );
    }
}

#[test]
fn a_lifetime_no_reading_accepts_states_none_rather_than_a_guess() {
    for lifetime in ["-1", "1.5", r#""soon""#, r#""""#, "true", "null"] {
        let body = format!(r#"{{"access_token":"t","expires_in":{lifetime}}}"#);
        let (_, lapses) = read_token(body.as_bytes(), now()).expect(&body);
        assert_eq!(lapses, None, "{lifetime}");
    }
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
    let error = read_token(br#"{"expires_in":"3599"}"#, now()).expect_err("a refusal");
    assert!(error.to_string().contains("access_token"), "{error}");
    assert!(read_token(b"not json", now()).is_err());
}
