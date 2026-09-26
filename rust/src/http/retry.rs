//! What every retrying client shares: the budget, the backoff, the
//! `Retry-After` reading and the two verdicts on a failure.
//!
//! One attempt is the rule; a retry happens only for a transport failure or
//! for an answer that says "not now", and only while the client can pay for
//! it. The pieces here decide when, how long, and whether at all; the client
//! that owns a request decides what to send again. The S3 client and the HTTP
//! client read the same rules, so a store and an origin server are retried
//! alike.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

/// Base of the exponential backoff between attempts.
pub(crate) const RETRY_BACKOFF: Duration = Duration::from_millis(50);
/// The longest a retry ever waits, however many attempts precede it.
pub(crate) const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(20);
/// The longest a `Retry-After` a server sent is honoured for.
///
/// A server under load may ask for minutes. Waiting that long inside a call
/// nobody can cancel is worse than failing and letting the caller decide, so
/// anything past this is treated as "not now" rather than as an instruction.
pub(crate) const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);
/// Tokens a client starts with, and never exceeds.
pub(crate) const RETRY_TOKENS: i64 = 500;
/// What one retry costs, so a client whose requests are all failing runs out.
pub(crate) const RETRY_COST: i64 = 5;
/// What a first-attempt success refunds.
pub(crate) const RETRY_REFUND: i64 = 1;

/// How long a client may spend on retries before it stops making them.
///
/// Doubling spreads one client's own attempts, and does nothing about the
/// other hundred that failed at the same instant: when a server is refusing
/// broadly, every client retrying every request turns a partial outage into a
/// worse one. A budget is what makes the client's total retry load bounded
/// rather than proportional to its failure rate. Each retry costs
/// [`RETRY_COST`] tokens, a request that succeeds without one refunds
/// [`RETRY_REFUND`], and a retry that succeeds gives its cost back, so a
/// healthy client always has budget and a client that is only failing runs out
/// and fails fast.
#[derive(Debug)]
pub(crate) struct RetryBudget {
    tokens: AtomicI64,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self {
            tokens: AtomicI64::new(RETRY_TOKENS),
        }
    }
}

impl RetryBudget {
    /// Take the price of one retry, or refuse it.
    pub(crate) fn withdraw(&self) -> bool {
        self.tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                (held >= RETRY_COST).then_some(held - RETRY_COST)
            })
            .is_ok()
    }

    /// Put `tokens` back, never above where the budget started.
    pub(crate) fn refund(&self, tokens: i64) {
        let _ = self
            .tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some((held + tokens).min(RETRY_TOKENS))
            });
    }

    /// What is left, for the counters to report.
    pub(crate) fn remaining(&self) -> i64 {
        self.tokens.load(Ordering::Relaxed)
    }
}

/// The window attempt `attempt + 1` is drawn from: doubling, and capped.
pub(crate) fn backoff(attempt: u32) -> Duration {
    let steps = attempt.saturating_sub(1).min(6);
    RETRY_BACKOFF
        .saturating_mul(1_u32 << steps)
        .min(RETRY_BACKOFF_CAP)
}

/// How long to wait before attempt `attempt + 1`.
///
/// A `Retry-After` is an instruction and is waited out as given. Anything
/// else is a *window*, [`backoff`]'s, and the wait is drawn uniformly from
/// it: doubling alone puts every client that failed at the same instant back
/// on the wire at the same instant, which is the herd the backoff exists to
/// prevent. The draw is a hash of `jitter`, the client's own counter started
/// by [`fresh_jitter`] and advanced by every draw, rather than a random
/// number generator - no dependency, no global state, and a sequence a test
/// can predict.
pub(crate) fn delay(attempt: u32, asked: Option<Duration>, jitter: &AtomicU64) -> Duration {
    match asked {
        Some(asked) => asked,
        None => {
            let window = backoff(attempt);
            let span = u64::try_from(window.as_nanos()).unwrap_or(u64::MAX);
            let draw = crate::xxhash::xxh3(&jitter.fetch_add(1, Ordering::Relaxed).to_le_bytes());
            Duration::from_nanos(draw % span.saturating_add(1))
        }
    }
}

/// A starting point for one client's jitter, different from every other's.
///
/// Two clients in one process that fail at the same instant should not draw
/// the same delays, so each starts its counter somewhere of its own.
pub(crate) fn fresh_jitter() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
    crate::xxhash::xxh3(
        &[u64::from(std::process::id()), ordinal]
            .map(u64::to_le_bytes)
            .concat(),
    )
}

/// The `Retry-After` an answer asks for, when it asks for one this will wait.
///
/// `value` is the header's value, when the answer carried one. Delta-seconds
/// only - RFC 9110's `1*DIGIT`, so no sign and no fraction: the HTTP-date
/// spelling is legal and reading a date needs a clock this has no reason to
/// trust; a client that wants the date read has the typed header reader for
/// it. A value past [`RETRY_AFTER_CAP`] is "not now" rather than an
/// instruction, and answers `None`.
pub(crate) fn retry_after(value: Option<&str>) -> Option<Duration> {
    let digits = value?.trim();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seconds: u64 = digits.parse().ok()?;
    let asked = Duration::from_secs(seconds);
    (asked <= RETRY_AFTER_CAP).then_some(asked)
}

/// Whether a transport failure is worth another attempt.
///
/// A connection that never established, timed out, or was cut is; a request
/// the client itself could not form is not.
pub(crate) fn is_retryable_transport(error: &ureq::Error) -> bool {
    matches!(
        error,
        ureq::Error::Io(_)
            | ureq::Error::Timeout(_)
            | ureq::Error::ConnectionFailed
            | ureq::Error::HostNotFound
    )
}

/// Whether a read failure is the transport's rather than the server's verdict.
///
/// The generic kind is included deliberately: a client library reports a
/// severed connection in more than one shape, and mistaking one for a decoding
/// failure costs the whole transfer where mistaking it the other way costs one
/// bounded re-open.
pub(crate) fn is_resumable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::Other
    )
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/retry.rs` pins and a caller cannot reach.
    //!
    //! The schedule, the budget arithmetic and the `Retry-After` cap are
    //! settled before a request goes out, so they are pinned with no server
    //! to answer. Every item forwards to the real one, so the module stays
    //! exactly as private as it was.
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;

    /// The pause before the first retry; each further one doubles it.
    pub const RETRY_BACKOFF: Duration = super::RETRY_BACKOFF;
    /// The longest a retry ever waits.
    pub const RETRY_BACKOFF_CAP: Duration = super::RETRY_BACKOFF_CAP;
    /// The longest a `Retry-After` is honoured for.
    pub const RETRY_AFTER_CAP: Duration = super::RETRY_AFTER_CAP;
    /// Tokens a budget starts with.
    pub const RETRY_TOKENS: i64 = super::RETRY_TOKENS;
    /// What one retry withdraws.
    pub const RETRY_COST: i64 = super::RETRY_COST;
    /// What a first-attempt success refunds.
    pub const RETRY_REFUND: i64 = super::RETRY_REFUND;

    /// How long the client waits before attempt `attempt`.
    pub fn backoff(attempt: u32) -> Duration {
        super::backoff(attempt)
    }

    /// The delay before attempt `attempt + 1`, jittered from `jitter` when
    /// nothing was asked for.
    pub fn delay(attempt: u32, asked: Option<Duration>, jitter: &AtomicU64) -> Duration {
        super::delay(attempt, asked, jitter)
    }

    /// A jitter starting point no other draw shares.
    pub fn fresh_jitter() -> u64 {
        super::fresh_jitter()
    }

    /// The wait a `Retry-After` value asks for, when it will be honoured.
    pub fn retry_after(value: Option<&str>) -> Option<Duration> {
        super::retry_after(value)
    }

    /// Whether a transport failure is retried.
    pub fn is_retryable_transport(error: &ureq::Error) -> bool {
        super::is_retryable_transport(error)
    }

    /// Whether a body read failure re-opens the transfer.
    pub fn is_resumable(error: &std::io::Error) -> bool {
        super::is_resumable(error)
    }

    /// One client's retry budget.
    #[derive(Debug, Default)]
    pub struct RetryBudget(super::RetryBudget);

    impl RetryBudget {
        /// Pay for one retry, or refuse it.
        pub fn withdraw(&self) -> bool {
            self.0.withdraw()
        }

        /// Give `tokens` back, capped at the start.
        pub fn refund(&self, tokens: i64) {
            self.0.refund(tokens);
        }

        /// What is left.
        pub fn remaining(&self) -> i64 {
            self.0.remaining()
        }
    }
}
