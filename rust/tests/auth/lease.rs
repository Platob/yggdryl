//! `rust/src/auth/lease.rs`: a value that lapses, held until shortly before
//! it does, and the expiry spellings.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::internals::auth_lease::{
    Expiring, Lease, instant, instant_from_millis, iso8601, lapses_within,
};

const WINDOW: Duration = Duration::from_secs(15 * 60);
const PAUSE: Duration = Duration::from_secs(30);
const HOLD: Duration = Duration::from_secs(300);

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds)
}

/// A token and when it lapses: the value the lease is proven over.
#[derive(Clone, Debug)]
struct Token {
    value: String,
    expires_at: Option<SystemTime>,
}

impl Token {
    fn new(value: impl Into<String>, expires_at: Option<SystemTime>) -> Self {
        Self {
            value: value.into(),
            expires_at,
        }
    }

    fn value(&self) -> &str {
        &self.value
    }
}

impl Expiring for Token {
    fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

fn lease() -> Lease<Token> {
    Lease::new("token", WINDOW, PAUSE, HOLD)
}

fn refusal(text: &str) -> yggdryl::Error {
    yggdryl::Error::Io(std::io::Error::other(text.to_owned()))
}

#[test]
fn a_held_value_is_answered_until_it_nears_its_end() {
    let lease = lease();
    let obtained = Cell::new(0);
    let obtain = || {
        obtained.set(obtained.get() + 1);
        Ok(Some(Token::new(
            format!("token-{}", obtained.get()),
            Some(at(3600)),
        )))
    };
    assert_eq!(
        lease.get(at(0), obtain).unwrap().unwrap().value(),
        "token-1"
    );
    assert_eq!(
        lease.get(at(60), obtain).unwrap().unwrap().value(),
        "token-1"
    );
    assert_eq!(obtained.get(), 1, "answered from what is held");
    // Fifteen minutes before the end it is replaced.
    assert_eq!(
        lease
            .get(at(3600 - 15 * 60), obtain)
            .unwrap()
            .unwrap()
            .value(),
        "token-2"
    );
    assert_eq!(obtained.get(), 2);
    assert_eq!(lease.peek().unwrap().value(), "token-2");
}

#[test]
fn a_value_without_an_end_is_never_replaced() {
    let lease = lease();
    let obtained = Cell::new(0);
    let obtain = || {
        obtained.set(obtained.get() + 1);
        Ok(Some(Token::new("forever", None)))
    };
    for seconds in [0, 86_400, 10 * 86_400] {
        assert_eq!(
            lease.get(at(seconds), obtain).unwrap().unwrap().value(),
            "forever"
        );
    }
    assert_eq!(obtained.get(), 1);
}

#[test]
fn a_failed_refresh_keeps_a_value_that_still_stands_and_pauses() {
    let lease = lease();
    let first = || Ok(Some(Token::new("first", Some(at(3600)))));
    lease.get(at(0), first).unwrap();
    let calls = Cell::new(0);
    let failing = || {
        calls.set(calls.get() + 1);
        Err(refusal("the endpoint is down"))
    };
    // Inside the window the failure is logged and the held value answered.
    let inside = at(3600 - 10 * 60);
    assert_eq!(
        lease.get(inside, failing).unwrap().unwrap().value(),
        "first"
    );
    assert_eq!(calls.get(), 1);
    // Within the pause, nothing is tried again.
    assert_eq!(
        lease
            .get(inside + Duration::from_secs(5), failing)
            .unwrap()
            .unwrap()
            .value(),
        "first"
    );
    assert_eq!(calls.get(), 1, "the pause holds");
    // After the pause, it is tried again.
    lease.get(inside + PAUSE, failing).unwrap();
    assert_eq!(calls.get(), 2);
    // Once the value has lapsed, the failure is the answer.
    let lapsed = at(3600 + 1);
    let error = lease.get(lapsed + PAUSE, failing).unwrap_err();
    assert!(
        error.to_string().contains("the endpoint is down"),
        "{error}"
    );
}

#[test]
fn a_failed_first_obtaining_is_answered_again_inside_the_pause() {
    let lease = lease();
    let calls = Cell::new(0);
    let failing = || {
        calls.set(calls.get() + 1);
        Err(refusal("nobody answers"))
    };
    assert!(lease.get(at(0), failing).is_err());
    let again = lease.get(at(1), failing).unwrap_err();
    assert!(again.to_string().contains("nobody answers"), "{again}");
    assert_eq!(calls.get(), 1, "the failure is repeated, not the obtaining");
    assert!(lease.get(at(31), failing).is_err());
    assert_eq!(calls.get(), 2, "after the pause it is tried again");
    // Success clears the failure.
    let succeeding = || Ok(Some(Token::new("now", None)));
    assert_eq!(
        lease.get(at(62), succeeding).unwrap().unwrap().value(),
        "now"
    );
    assert_eq!(lease.get(at(63), failing).unwrap().unwrap().value(), "now");
    assert_eq!(calls.get(), 2);
}

#[test]
fn nothing_to_obtain_holds_for_the_hold_or_until_invalidated() {
    let lease = lease();
    let calls = Cell::new(0);
    let nothing = || {
        calls.set(calls.get() + 1);
        Ok(None)
    };
    assert!(lease.get(at(0), nothing).unwrap().is_none());
    assert!(lease.get(at(1), nothing).unwrap().is_none());
    assert_eq!(calls.get(), 1, "nothing is remembered for the hold");
    assert!(lease.get(at(300), nothing).unwrap().is_none());
    assert_eq!(calls.get(), 2, "after the hold it is looked for again");
    lease.invalidate();
    assert!(lease.get(at(301), nothing).unwrap().is_none());
    assert_eq!(calls.get(), 3, "invalidating looks again at once");
    assert!(lease.peek().is_none());
}

#[test]
fn a_value_obtained_already_inside_its_window_is_used_for_the_pause() {
    let lease = lease();
    let calls = Cell::new(0);
    // A fifteen-minute session is stale the moment it arrives.
    let short = || {
        calls.set(calls.get() + 1);
        Ok(Some(Token::new("short", Some(at(900)))))
    };
    assert_eq!(lease.get(at(0), short).unwrap().unwrap().value(), "short");
    assert_eq!(lease.get(at(1), short).unwrap().unwrap().value(), "short");
    assert_eq!(
        calls.get(),
        1,
        "used for the pause rather than obtained per call"
    );
    lease.get(at(31), short).unwrap();
    assert_eq!(calls.get(), 2, "obtained again after the pause");
}

#[test]
fn a_failure_once_the_value_has_lapsed_is_paused_too() {
    let lease = lease();
    lease
        .get(at(0), || Ok(Some(Token::new("gone", Some(at(60))))))
        .unwrap();
    let calls = Cell::new(0);
    let failing = || {
        calls.set(calls.get() + 1);
        Err(refusal("still down"))
    };
    assert!(lease.get(at(120), failing).is_err());
    assert!(lease.get(at(121), failing).is_err());
    assert_eq!(
        calls.get(),
        1,
        "the failure is answered again inside the pause"
    );
    assert!(lease.get(at(151), failing).is_err());
    assert_eq!(calls.get(), 2);
}

#[test]
fn a_held_value_that_lapses_with_nothing_to_replace_it_becomes_nothing() {
    let lease = lease();
    lease
        .get(at(0), || Ok(Some(Token::new("short", Some(at(60))))))
        .unwrap();
    // Still standing, and nothing new: keep it, pause.
    assert_eq!(
        lease.get(at(30), || Ok(None)).unwrap().unwrap().value(),
        "short"
    );
    // Lapsed, and nothing new: nothing.
    assert!(lease.get(at(120), || Ok(None)).unwrap().is_none());
    assert!(lease.peek().is_none());
}

#[test]
fn lapses_within_reads_the_window_off_the_value() {
    let bearer = Token::new("t", Some(at(3600)));
    assert!(!lapses_within(&bearer, at(0), WINDOW));
    assert!(lapses_within(&bearer, at(3600 - 15 * 60), WINDOW));
    assert!(lapses_within(&bearer, at(3600), Duration::ZERO));
    assert!(!lapses_within(&bearer, at(3599), Duration::ZERO));
    assert!(!lapses_within(&Token::new("t", None), at(0), WINDOW));
    assert_eq!(bearer.expires_at(), Some(at(3600)));
}

#[test]
fn every_expiry_spelling_the_tools_write_reads_as_one_instant() {
    let epoch = instant("1970-01-01T00:00:00Z").unwrap();
    assert_eq!(epoch, UNIX_EPOCH);
    let later = UNIX_EPOCH + Duration::from_secs(1_369_353_600);
    assert_eq!(instant("2013-05-24T00:00:00Z"), Some(later));
    assert_eq!(instant("2013-05-24T00:00:00.000Z"), Some(later));
    assert_eq!(
        instant("2013-05-24T02:00:00+02:00"),
        Some(later),
        "an offset"
    );
    assert_eq!(
        instant("2013-05-24T00:00:00UTC"),
        Some(later),
        "the CLI cache's spelling"
    );
    assert_eq!(instant("2013-05-24T00:00:00"), Some(later), "naive is UTC");
    assert_eq!(instant("  2013-05-24T00:00:00Z  "), Some(later), "trimmed");
    assert_eq!(instant("not an instant"), None);
    assert_eq!(instant(""), None);
    assert_eq!(
        instant_from_millis(1_369_353_600_500),
        Some(later + Duration::from_millis(500))
    );
    assert_eq!(iso8601(later), "2013-05-24T00:00:00Z");
    assert_eq!(
        instant(&iso8601(at(12_345))),
        Some(at(12_345)),
        "round trip"
    );
}
