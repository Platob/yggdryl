//! UTC wall-clock timestamps for request/response timing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The current UTC time as Unix-epoch **seconds** (`0.0` if the clock is before
/// the epoch).
pub(crate) fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A shared, settable UTC timestamp (Unix-epoch seconds) — an `f64` stored in an
/// [`AtomicU64`] via its bit pattern, so an [`HttpStream`](crate::HttpStream) and
/// the [`HttpResponse`](crate::HttpResponse) that returned it can read the same
/// "connection done" instant once the body reaches EOF or is closed. Defaults to
/// `0.0` (unset).
#[derive(Debug, Clone, Default)]
pub(crate) struct Instant(Arc<AtomicU64>);

impl Instant {
    /// A fresh, unset instant reading `0.0`.
    pub(crate) fn new() -> Instant {
        Instant(Arc::new(AtomicU64::new(0)))
    }

    /// The stored timestamp in Unix-epoch seconds (`0.0` when unset).
    pub(crate) fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::SeqCst))
    }

    /// Stamps the current time, but only if still unset (so the first "done"
    /// wins and a later `close` after EOF does not overwrite it).
    pub(crate) fn stamp_once(&self) {
        if self.get() == 0.0 {
            self.0.store(now_secs().to_bits(), Ordering::SeqCst);
        }
    }
}
