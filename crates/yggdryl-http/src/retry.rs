//! The [`RetryConfig`] policy and the shared pool / cache constants.

use std::time::Duration;

/// The default idle-connection pool size.
pub(crate) const DEFAULT_POOL: usize = 16;

/// The most recently-streamed bytes [`HttpStream`](crate::HttpStream) keeps for a
/// seek-back (4 MiB).
pub(crate) const CACHE_LIMIT: usize = 4 * 1024 * 1024;

/// How [`HttpSession`](crate::HttpSession) retries transient failures: rate-limit /
/// unavailable statuses (429 / 502 / 503 / 504, honouring `Retry-After`) and lost
/// connections, with capped exponential backoff. A retried request resumes a
/// streamed [`HttpStream`](crate::HttpStream) from its current cursor via a `Range`
/// re-request.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries after the first attempt (default `3`).
    pub max_retries: u32,
    /// The base backoff delay, doubled each attempt (default `200ms`).
    pub base_delay: Duration,
    /// The cap on any single backoff delay (default `10s`).
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryConfig {
    /// Whether a response status is a transient one worth retrying.
    pub(crate) fn retryable_status(&self, status: u16) -> bool {
        matches!(status, 429 | 502 | 503 | 504)
    }

    /// The delay before the next attempt: a `Retry-After` value if the server
    /// gave one, else capped exponential backoff.
    pub(crate) fn backoff(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(retry_after) = retry_after {
            return retry_after.min(self.max_delay);
        }
        let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }
}
