//! [`RetryConfig`] — exponential-backoff retry policy.

use std::time::Duration;

/// Retry policy applied to transient HTTP failures.
///
/// Retried statuses: `429` (honours `Retry-After`), `502`, `503`, `504`.
/// Transport failures (connection reset, unexpected EOF) are also retried.
///
/// Delay formula: `min(base_delay × 2ⁿ, max_delay)` where `n` is the
/// zero-indexed attempt number.
///
/// ```
/// use yggdryl_http::RetryConfig;
/// use std::time::Duration;
///
/// let r = RetryConfig::new(3, Duration::from_millis(100), Duration::from_secs(5));
/// assert_eq!(r.delay(0), Duration::from_millis(100));
/// assert_eq!(r.delay(1), Duration::from_millis(200));
/// assert_eq!(r.delay(10), Duration::from_secs(5)); // capped
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the first try).
    pub max_retries: u32,
    /// Initial delay before the first retry.
    pub base_delay: Duration,
    /// Upper cap on the backoff delay.
    pub max_delay: Duration,
}

impl RetryConfig {
    /// Constructs a retry policy.
    pub fn new(max_retries: u32, base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_retries,
            base_delay,
            max_delay,
        }
    }

    /// The default policy: 3 retries, 500 ms base, 30 s cap.
    pub fn default_policy() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }

    /// Computes the capped exponential backoff for attempt `n` (0-indexed).
    pub fn delay(&self, n: u32) -> Duration {
        // `saturating_mul` prevents overflow on very large n.
        let exp = self.base_delay.saturating_mul(2u32.saturating_pow(n));
        exp.min(self.max_delay)
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self::default_policy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_capped_exponential() {
        let r = RetryConfig::new(3, Duration::from_millis(100), Duration::from_millis(500));
        assert_eq!(r.delay(0), Duration::from_millis(100));
        assert_eq!(r.delay(1), Duration::from_millis(200));
        assert_eq!(r.delay(2), Duration::from_millis(400));
        assert_eq!(r.delay(3), Duration::from_millis(500)); // capped
        assert_eq!(r.delay(100), Duration::from_millis(500)); // still capped
    }
}
