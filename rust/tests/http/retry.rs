//! `rust/src/http/retry.rs`: the schedule, the budget and the two verdicts.
//!
//! Everything here is settled before a request goes out, so it is pinned with
//! no server to answer: how long attempt `n` waits, how many retries a client
//! can pay for, which `Retry-After` is honoured, and which failure is worth
//! another attempt. The S3 client reads the same rules, and
//! `rust/tests/s3/client.rs` pins that it does.

use std::sync::atomic::AtomicU64;
use std::time::Duration;

use yggdryl::internals::http_retry::{
    RETRY_BACKOFF, RETRY_BACKOFF_CAP, RETRY_COST, RETRY_REFUND, RETRY_TOKENS, RetryBudget, backoff,
    delay, fresh_jitter, is_resumable, is_retryable_transport, is_unsent, retry_after,
};

#[test]
fn the_backoff_doubles_from_its_base_and_stops_doubling_after_six_steps() {
    assert_eq!(RETRY_BACKOFF, Duration::from_millis(50));
    assert_eq!(backoff(0), RETRY_BACKOFF);
    assert_eq!(backoff(1), RETRY_BACKOFF);
    let mut expected = RETRY_BACKOFF;
    for attempt in 2..=7 {
        expected *= 2;
        assert_eq!(backoff(attempt), expected, "attempt {attempt}");
    }
    // Six doublings is the ceiling: 3.2 seconds, under the cap, and every
    // later attempt waits the same.
    assert_eq!(backoff(7), Duration::from_millis(3_200));
    assert_eq!(backoff(8), backoff(7));
    assert_eq!(backoff(u32::MAX), backoff(7));
    assert!(backoff(u32::MAX) <= RETRY_BACKOFF_CAP);
    assert_eq!(RETRY_BACKOFF_CAP, Duration::from_secs(20));
}

#[test]
fn a_delay_is_what_was_asked_for_else_a_jittered_draw_inside_the_window() {
    let jitter = AtomicU64::new(fresh_jitter());
    assert_eq!(
        delay(3, Some(Duration::from_secs(4)), &jitter),
        Duration::from_secs(4)
    );
    for attempt in 1..=8 {
        let window = backoff(attempt);
        for _ in 0..16 {
            let drawn = delay(attempt, None, &jitter);
            assert!(drawn <= window, "attempt {attempt}: {drawn:?} > {window:?}");
        }
    }
    // Sixteen draws from one window do not all land on one value: the counter
    // advances and the hash spreads it.
    let draws: std::collections::BTreeSet<Duration> =
        (0..16).map(|_| delay(6, None, &jitter)).collect();
    assert!(draws.len() > 1);
}

#[test]
fn two_fresh_jitters_never_start_at_the_same_place() {
    let first = fresh_jitter();
    let second = fresh_jitter();
    assert_ne!(first, second);
}

#[test]
fn the_budget_pays_for_retries_until_it_runs_out_and_refunds_never_exceed_the_start() {
    assert_eq!((RETRY_TOKENS, RETRY_COST, RETRY_REFUND), (500, 5, 1));
    let budget = RetryBudget::default();
    assert_eq!(budget.remaining(), RETRY_TOKENS);

    let affordable = RETRY_TOKENS / RETRY_COST;
    for retry in 0..affordable {
        assert!(budget.withdraw(), "retry {retry} refused early");
    }
    assert_eq!(budget.remaining(), 0);
    assert!(!budget.withdraw(), "an empty budget still paid");
    assert_eq!(budget.remaining(), 0);

    // A first-attempt success gives one token back, not enough for a retry;
    // a retry that succeeded gives its whole cost back.
    budget.refund(RETRY_REFUND);
    assert_eq!(budget.remaining(), RETRY_REFUND);
    assert!(!budget.withdraw());
    budget.refund(RETRY_COST);
    assert_eq!(budget.remaining(), RETRY_REFUND + RETRY_COST);
    assert!(budget.withdraw());
    assert_eq!(budget.remaining(), RETRY_REFUND);

    budget.refund(RETRY_TOKENS * 10);
    assert_eq!(
        budget.remaining(),
        RETRY_TOKENS,
        "a refund overflowed the start"
    );
}

#[test]
fn retry_after_reads_delta_seconds_or_a_date_and_nothing_else() {
    assert_eq!(retry_after(None), None);
    assert_eq!(retry_after(Some("0")), Some(Duration::ZERO));
    assert_eq!(retry_after(Some("3")), Some(Duration::from_secs(3)));
    assert_eq!(retry_after(Some("  7 ")), Some(Duration::from_secs(7)));
    // No cap here: whether a long pause is waited is each client's call.
    assert_eq!(retry_after(Some("3600")), Some(Duration::from_secs(3600)));
    // A date that has passed asks for no pause at all.
    assert_eq!(
        retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
        Some(Duration::ZERO)
    );
    let far = retry_after(Some("Fri, 01 Jan 2100 00:00:00 GMT")).expect("a date reads");
    assert!(far > Duration::from_secs(86_400 * 365), "{far:?}");
    // Neither spelling: a sign, a fraction, text, nothing, an overflow.
    for malformed in ["-1", "+3", "2.5", "soon", "", "99999999999999999999999"] {
        assert_eq!(retry_after(Some(malformed)), None, "{malformed:?}");
    }
}

#[test]
fn a_transport_failure_is_retried_and_a_malformed_request_is_not() {
    for retryable in [
        ureq::Error::Io(std::io::Error::other("reset")),
        ureq::Error::Timeout(ureq::Timeout::Connect),
        ureq::Error::Timeout(ureq::Timeout::RecvBody),
        ureq::Error::ConnectionFailed,
        ureq::Error::HostNotFound,
    ] {
        assert!(is_retryable_transport(&retryable), "{retryable:?}");
    }
    for verdict in [
        ureq::Error::StatusCode(503),
        ureq::Error::BadUri("::".to_owned()),
        ureq::Error::RedirectFailed,
        ureq::Error::TooManyRedirects,
        ureq::Error::BodyExceedsLimit(1),
        ureq::Error::InvalidProxyUrl,
        ureq::Error::Tls("refused"),
    ] {
        assert!(!is_retryable_transport(&verdict), "{verdict:?}");
    }
}

#[test]
fn a_request_is_unsent_only_when_no_connection_took_it() {
    for unsent in [
        ureq::Error::HostNotFound,
        ureq::Error::ConnectionFailed,
        ureq::Error::Timeout(ureq::Timeout::Resolve),
        ureq::Error::Timeout(ureq::Timeout::Connect),
        ureq::Error::Io(std::io::ErrorKind::ConnectionRefused.into()),
    ] {
        assert!(is_unsent(&unsent), "{unsent:?}");
    }
    for sent in [
        ureq::Error::Timeout(ureq::Timeout::SendRequest),
        ureq::Error::Timeout(ureq::Timeout::RecvResponse),
        ureq::Error::Io(std::io::ErrorKind::ConnectionReset.into()),
        ureq::Error::Io(std::io::ErrorKind::BrokenPipe.into()),
        ureq::Error::StatusCode(503),
    ] {
        assert!(!is_unsent(&sent), "{sent:?}");
    }
}

#[test]
fn a_severed_read_resumes_and_a_verdict_on_the_bytes_does_not() {
    use std::io::ErrorKind;

    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
        ErrorKind::UnexpectedEof,
        ErrorKind::TimedOut,
        ErrorKind::Interrupted,
        ErrorKind::Other,
    ] {
        assert!(is_resumable(&std::io::Error::from(kind)), "{kind:?}");
    }
    assert!(is_resumable(&std::io::Error::other(
        "a boxed transport failure"
    )));
    for kind in [
        ErrorKind::InvalidData,
        ErrorKind::InvalidInput,
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::Unsupported,
        ErrorKind::WriteZero,
    ] {
        assert!(!is_resumable(&std::io::Error::from(kind)), "{kind:?}");
    }
}
