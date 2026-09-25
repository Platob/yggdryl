//! A value that lapses, held until shortly before it does.
//!
//! A temporary credential set, a bearer token, a session a role was traded
//! for: each is obtained at a cost, accepted for a while, and refused after.
//! [`Lease`] holds one such value behind a lock and answers it until it is
//! near its end, obtains another then, and - this is the part every fast
//! failing client gets wrong - keeps the one it has when obtaining another
//! fails while the one it has still stands, tries again after a pause
//! rather than on every request, and only refuses once the value has
//! actually lapsed. What obtaining a value means is the holder's; when it
//! lapses is the value's, through [`Expiring`].

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "s3")]
use super::Secret;
use crate::{Error, Result, TimeUnit};

/// A value that stops being accepted at an instant, or never.
pub trait Expiring {
    /// When the value lapses; `None` is a value that does not.
    fn expires_at(&self) -> Option<SystemTime>;
}

/// A bearer token and when it lapses: what Google and Azure each hand a
/// client.
#[cfg(feature = "s3")]
#[derive(Clone, Debug)]
pub struct Bearer {
    value: Secret,
    expires_at: Option<SystemTime>,
}

#[cfg(feature = "s3")]
impl Bearer {
    /// `value`, lapsing at `expires_at` when it does.
    pub fn new(value: impl Into<Secret>, expires_at: Option<SystemTime>) -> Self {
        Self {
            value: value.into(),
            expires_at,
        }
    }

    /// The token, for the request that carries it.
    pub fn value(&self) -> &str {
        self.value.expose()
    }
}

#[cfg(feature = "s3")]
impl Expiring for Bearer {
    fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

/// What is held between two obtainings.
struct State<T> {
    held: Option<T>,
    /// Obtaining answered that there is nothing to obtain: `None` is the
    /// answer until this instant, or until the lease is invalidated.
    none_until: Option<SystemTime>,
    /// Obtaining failed, or answered a value already near its end; until
    /// this instant nothing is obtained again.
    retry_after: Option<SystemTime>,
    failure: Option<String>,
}

impl<T> Default for State<T> {
    fn default() -> Self {
        Self {
            held: None,
            none_until: None,
            retry_after: None,
            failure: None,
        }
    }
}

/// One expiring value, obtained on demand and refreshed before it lapses.
pub struct Lease<T> {
    /// What is held, for the log line that says it was kept.
    what: &'static str,
    /// Obtain another this long before the held value lapses.
    window: Duration,
    /// After a failed obtaining, wait this long before trying again.
    pause: Duration,
    /// After obtaining found nothing, wait this long before looking again.
    hold: Duration,
    state: Mutex<State<T>>,
}

impl<T: Expiring + Clone> Lease<T> {
    /// An empty lease over `what`, refreshing `window` before expiry,
    /// pausing `pause` after a failure and `hold` after finding nothing.
    pub const fn new(
        what: &'static str,
        window: Duration,
        pause: Duration,
        hold: Duration,
    ) -> Self {
        Self {
            what,
            window,
            pause,
            hold,
            state: Mutex::new(State {
                held: None,
                none_until: None,
                retry_after: None,
                failure: None,
            }),
        }
    }

    /// The value to use at `now`, obtaining one through `obtain` when none
    /// is held or the held one nears its end.
    ///
    /// `obtain` runs under the lease's lock, so two requests never obtain at
    /// once and every other request waits for the one obtaining; it must
    /// not reach back into the lease. It answers `Ok(None)` when there is
    /// nothing to obtain, which is then the answer for the hold, or until
    /// [`Self::invalidate`]. Its failure is the answer only when nothing
    /// usable is held: a held value that still stands is answered instead,
    /// with the failure logged and the next try deferred by the pause - and
    /// a failure is deferred the same way once nothing stands, so a source
    /// that is down costs a request its answer rather than every request a
    /// walk. A value obtained already inside its own window - a session
    /// shorter than the window - is used for the pause before another is
    /// asked for.
    ///
    /// # Errors
    ///
    /// What `obtain` refused, once nothing held can stand in for it.
    pub fn get<F>(&self, now: SystemTime, obtain: F) -> Result<Option<T>>
    where
        F: FnOnce() -> Result<Option<T>>,
    {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let paused = state.retry_after.is_some_and(|at| now < at);
        if let Some(held) = state.held.clone() {
            if !lapses_within(&held, now, self.window) {
                return Ok(Some(held));
            }
            let expired = lapses_within(&held, now, Duration::ZERO);
            if paused && !expired {
                return Ok(Some(held));
            }
            if paused {
                if let Some(failure) = &state.failure {
                    return Err(repeated(failure));
                }
            }
            return match obtain() {
                Ok(Some(fresh)) => Ok(Some(self.adopt(&mut state, fresh, now))),
                Ok(None) if expired => {
                    state.held = None;
                    state.none_until = Some(now + self.hold);
                    Ok(None)
                }
                Ok(None) => {
                    state.retry_after = Some(now + self.pause);
                    Ok(Some(held))
                }
                Err(error) => {
                    state.retry_after = Some(now + self.pause);
                    state.failure = Some(error.to_string());
                    if expired {
                        return Err(error);
                    }
                    log::warn!(
                        "keeping the {} in hand, which still stands: obtaining another failed: {error}",
                        self.what
                    );
                    Ok(Some(held))
                }
            };
        }
        if state.none_until.is_some_and(|until| now < until) {
            return Ok(None);
        }
        if paused {
            if let Some(failure) = &state.failure {
                return Err(repeated(failure));
            }
        }
        match obtain() {
            Ok(Some(found)) => Ok(Some(self.adopt(&mut state, found, now))),
            Ok(None) => {
                state.none_until = Some(now + self.hold);
                Ok(None)
            }
            Err(error) => {
                state.retry_after = Some(now + self.pause);
                state.failure = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Hold `fresh`, and hold off obtaining another when it is already near
    /// its end.
    fn adopt(&self, state: &mut State<T>, fresh: T, now: SystemTime) -> T {
        state.retry_after = lapses_within(&fresh, now, self.window).then(|| now + self.pause);
        state.failure = None;
        state.none_until = None;
        state.held = Some(fresh.clone());
        fresh
    }

    /// The value held, without obtaining one.
    pub fn peek(&self) -> Option<T> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .held
            .clone()
    }

    /// Forget everything held and learned, so the next [`Self::get`]
    /// obtains again.
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *state = State::default();
    }
}

/// Whether `value` lapses within `window` of `now`.
pub fn lapses_within<T: Expiring>(value: &T, now: SystemTime, window: Duration) -> bool {
    value
        .expires_at()
        .is_some_and(|expiry| now.checked_add(window).is_none_or(|edge| edge >= expiry))
}

/// The failure answered again inside the pause.
fn repeated(failure: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        failure.to_owned(),
    ))
}

/// The instant an expiry states, in any spelling a cloud's tools write one.
///
/// ISO 8601 with `Z` or an offset is what every service answers; the AWS
/// CLI's own caches write the offset as a trailing `UTC`, and a naive reading
/// is taken as UTC, which is the only zone any of them means. Anything else
/// answers `None`, so the value is treated as long-lived rather than lapsing
/// at a guessed instant.
pub fn instant(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let text = text
        .strip_suffix("UTC")
        .map_or_else(|| text.to_owned(), |head| format!("{head}Z"));
    let (count, unit) = match crate::temporal::parse_timestamp(&text) {
        Ok((count, unit, _)) => (count, unit),
        Err(_) => crate::temporal::parse_datetime(&text).ok()?,
    };
    instant_of(count, unit)
}

/// The instant `count` units since the epoch names.
fn instant_of(count: i64, unit: TimeUnit) -> Option<SystemTime> {
    let per = crate::temporal::per_second(unit)?;
    let seconds = count.div_euclid(per);
    let rest = count.rem_euclid(per);
    let nanos = (rest * (1_000_000_000 / per)).try_into().ok()?;
    let seconds = u64::try_from(seconds).ok()?;
    UNIX_EPOCH.checked_add(Duration::new(seconds, nanos))
}

/// The instant an epoch count in milliseconds names, which is how the SSO
/// portal states an expiry.
pub fn instant_from_millis(millis: i64) -> Option<SystemTime> {
    instant_of(millis, TimeUnit::Millisecond)
}

/// `instant` spelled as the services answer one: `YYYY-MM-DDThh:mm:ssZ`.
pub fn iso8601(instant: SystemTime) -> String {
    let seconds = instant
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day) =
        crate::timezone::civil_from_days(i64::try_from(seconds / 86_400).unwrap_or(i64::MAX));
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/auth/lease.rs` pins and a caller cannot reach.
    //!
    //! The lease is driven with a value a test builds and an `obtain` a test
    //! scripts; the expiry spellings are pinned by text. Each item forwards.
    pub use super::{Expiring, Lease};

    use std::time::{Duration, SystemTime};

    /// The instant an expiry states, or `None` for a spelling nothing reads.
    pub fn instant(text: &str) -> Option<SystemTime> {
        super::instant(text)
    }

    /// The instant an epoch count in milliseconds names.
    pub fn instant_from_millis(millis: i64) -> Option<SystemTime> {
        super::instant_from_millis(millis)
    }

    /// `instant` spelled as the services answer one.
    pub fn iso8601(instant: SystemTime) -> String {
        super::iso8601(instant)
    }

    /// Whether `value` lapses within `window` of `now`.
    pub fn lapses_within<T: Expiring>(value: &T, now: SystemTime, window: Duration) -> bool {
        super::lapses_within(value, now, window)
    }
}
