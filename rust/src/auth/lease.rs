//! A value that lapses, held until shortly before it does.
//!
//! A temporary credential set, a bearer token, a session a role was traded
//! for: each is obtained at a cost, accepted for a while, and refused after.
//! [`Lease`] holds one such value behind a lock and answers it until it is
//! near its end, obtains another then, and - this is the part every fast
//! failing client gets wrong - keeps the one it has when obtaining another
//! fails while the one it has still stands outside its mandatory window,
//! tries again after a pause rather than on every request, and refuses once
//! the value is inside that window. One request obtains at a time, outside
//! the lock: in the advisory window every other request answers the value
//! in hand meanwhile, and only one with nothing usable in hand waits for
//! it. What obtaining a value means is the holder's; when it lapses is the
//! value's, through [`Expiring`].

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "s3")]
use super::Secret;
use crate::{Error, Result, TimeUnit};

/// A value that stops being accepted at an instant, or never.
pub trait Expiring {
    /// When the value lapses; `None` is a value that does not.
    fn expires_at(&self) -> Option<SystemTime>;

    /// How long before it lapses this value is replaced, when its source
    /// keeps a window of its own - a sign-in whose sets last fifteen
    /// minutes cannot be replaced fifteen minutes ahead; `None` takes the
    /// lease's advisory window.
    fn refresh_window(&self) -> Option<Duration> {
        None
    }

    /// How long before it lapses a failed refresh is the answer rather
    /// than this value; `None` takes the lease's mandatory window. A value
    /// stating its own [`Self::refresh_window`] states none by default
    /// (`Duration::ZERO`): its source's window is the whole of its rule, so
    /// it is used until it lapses.
    fn mandatory_window(&self) -> Option<Duration> {
        self.refresh_window().map(|_| Duration::ZERO)
    }
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
    /// One request is obtaining, outside the lock; at most one ever is.
    /// Survives [`Lease::invalidate`], since that request is still running.
    refreshing: bool,
    /// Bumped by [`Lease::invalidate`]: an obtaining begun under another
    /// generation answers its own request and is held by nobody.
    generation: u64,
}

impl<T> State<T> {
    const fn empty(refreshing: bool, generation: u64) -> Self {
        Self {
            held: None,
            none_until: None,
            retry_after: None,
            failure: None,
            refreshing,
            generation,
        }
    }
}

/// What a request does next, read off the state under the lock.
enum Step<T> {
    Answer(Result<Option<T>>),
    /// Another request is obtaining and nothing usable is in hand.
    Wait,
    Obtain,
}

/// Clears the obtaining flag and wakes the waiters when `obtain` unwinds,
/// so a panicking source does not leave every later request waiting.
struct Obtaining<'a, T> {
    lease: &'a Lease<T>,
}

impl<T> Drop for Obtaining<'_, T> {
    fn drop(&mut self) {
        self.lease.lock().refreshing = false;
        self.lease.turn.notify_all();
    }
}

/// One expiring value, obtained on demand and refreshed before it lapses.
pub struct Lease<T> {
    /// What is held, for the log line that says it was kept.
    name: &'static str,
    /// Obtain another this long before the held value lapses; a failure
    /// here keeps the held value.
    advisory: Duration,
    /// Inside this long before the held value lapses a failed obtaining is
    /// the answer; capped at the value's advisory window.
    mandatory: Duration,
    /// After a failed obtaining, wait this long before trying again.
    retry_pause: Duration,
    /// After obtaining found nothing, wait this long before looking again.
    anonymous_hold: Duration,
    state: Mutex<State<T>>,
    /// Signalled when an obtaining ends, for the requests waiting on it.
    turn: Condvar,
}

impl<T> Lease<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T: Expiring + Clone> Lease<T> {
    /// An empty lease over `name`: a value is replaced `advisory` before it
    /// lapses, a failed replacement keeps it until `mandatory` before it
    /// lapses and is the answer inside that, a failure is tried again after
    /// `retry_pause`, and finding nothing is the answer for
    /// `anonymous_hold`. `Duration::ZERO` for `mandatory` keeps a value
    /// until the instant it lapses.
    pub const fn new(
        name: &'static str,
        advisory: Duration,
        mandatory: Duration,
        retry_pause: Duration,
        anonymous_hold: Duration,
    ) -> Self {
        Self {
            name,
            advisory,
            mandatory,
            retry_pause,
            anonymous_hold,
            state: Mutex::new(State::empty(false, 0)),
            turn: Condvar::new(),
        }
    }

    /// The value to use at `now`, obtaining one through `obtain` when none
    /// is held or the held one nears its end.
    ///
    /// At most one request runs `obtain` at a time, and it runs outside the
    /// lease's lock, so `obtain` may take as long as its source does; it
    /// must not reach back into the lease. While it runs, a request whose
    /// held value is in the advisory window but outside the mandatory one
    /// answers that value at once, and every other request waits for the
    /// outcome rather than obtaining again, so concurrent requests cost one
    /// obtaining per window. `obtain` answers `Ok(None)` when there is
    /// nothing to obtain, which is then the answer for the hold, or until
    /// [`Self::invalidate`]. Its failure is the answer only when nothing
    /// usable is held: a held value outside its mandatory window is
    /// answered instead, with the failure logged and the next try deferred
    /// by the pause - and a failure is deferred the same way once nothing
    /// usable stands, so a source that is down costs a request its answer
    /// rather than every request a walk. A value obtained already inside
    /// its own window - a session shorter than the window - is used for the
    /// pause before another is asked for, until it lapses.
    ///
    /// # Errors
    ///
    /// What `obtain` refused, once nothing held can stand in for it: none
    /// is held, or the held value is inside its mandatory window.
    pub fn get<F>(&self, now: SystemTime, obtain: F) -> Result<Option<T>>
    where
        F: FnOnce() -> Result<Option<T>>,
    {
        let mut state = self.lock();
        loop {
            match self.step(&state, now) {
                Step::Answer(answer) => return answer,
                Step::Wait => {
                    state = self
                        .turn
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Step::Obtain => break,
            }
        }
        state.refreshing = true;
        let generation = state.generation;
        drop(state);
        let obtaining = Obtaining { lease: self };
        let outcome = obtain();
        let mut state = self.lock();
        // Cleared here, under the lock the outcome is settled under, so no
        // request sees the flag set over a settled state, and no later
        // obtaining's flag is cleared by this one's guard.
        std::mem::forget(obtaining);
        state.refreshing = false;
        let answer = if state.generation == generation {
            self.settle(&mut state, now, outcome)
        } else {
            outcome
        };
        drop(state);
        self.turn.notify_all();
        answer
    }

    /// What a request at `now` does with `state` as it stands.
    fn step(&self, state: &State<T>, now: SystemTime) -> Step<T> {
        let paused = state.retry_after.is_some_and(|at| now < at);
        if let Some(held) = &state.held {
            if !lapses_within(held, now, self.advisory_of(held)) {
                return Step::Answer(Ok(Some(held.clone())));
            }
            let usable = !lapses_within(held, now, self.mandatory_of(held));
            if paused {
                match &state.failure {
                    Some(_) if usable => return Step::Answer(Ok(Some(held.clone()))),
                    Some(failure) => return Step::Answer(Err(repeated(failure))),
                    None if !lapses_within(held, now, Duration::ZERO) => {
                        return Step::Answer(Ok(Some(held.clone())));
                    }
                    None => {}
                }
            }
            if state.refreshing {
                return if usable {
                    Step::Answer(Ok(Some(held.clone())))
                } else {
                    Step::Wait
                };
            }
            return Step::Obtain;
        }
        if state.none_until.is_some_and(|until| now < until) {
            return Step::Answer(Ok(None));
        }
        if paused && let Some(failure) = &state.failure {
            return Step::Answer(Err(repeated(failure)));
        }
        if state.refreshing {
            return Step::Wait;
        }
        Step::Obtain
    }

    /// Record what obtaining at `now` answered, and answer the request that
    /// obtained.
    fn settle(
        &self,
        state: &mut State<T>,
        now: SystemTime,
        outcome: Result<Option<T>>,
    ) -> Result<Option<T>> {
        let Some(held) = state.held.clone() else {
            return match outcome {
                Ok(Some(found)) => Ok(Some(self.adopt(state, found, now))),
                Ok(None) => {
                    state.none_until = Some(now + self.anonymous_hold);
                    Ok(None)
                }
                Err(error) => {
                    state.retry_after = Some(now + self.retry_pause);
                    state.failure = Some(error.to_string());
                    Err(error)
                }
            };
        };
        match outcome {
            Ok(Some(fresh)) => Ok(Some(self.adopt(state, fresh, now))),
            Ok(None) if lapses_within(&held, now, Duration::ZERO) => {
                state.held = None;
                state.none_until = Some(now + self.anonymous_hold);
                Ok(None)
            }
            Ok(None) => {
                state.retry_after = Some(now + self.retry_pause);
                Ok(Some(held))
            }
            Err(error) => {
                state.retry_after = Some(now + self.retry_pause);
                state.failure = Some(error.to_string());
                if lapses_within(&held, now, self.mandatory_of(&held)) {
                    return Err(error);
                }
                log::warn!(
                    "keeping the {} in hand, which stands outside its mandatory window: obtaining another failed: {error}",
                    self.name
                );
                Ok(Some(held))
            }
        }
    }

    /// Hold `fresh`, and hold off obtaining another when it is already near
    /// its end.
    fn adopt(&self, state: &mut State<T>, fresh: T, now: SystemTime) -> T {
        state.retry_after =
            lapses_within(&fresh, now, self.advisory_of(&fresh)).then(|| now + self.retry_pause);
        state.failure = None;
        state.none_until = None;
        state.held = Some(fresh.clone());
        fresh
    }

    /// The window `value` is replaced in: its own, else the lease's.
    fn advisory_of(&self, value: &T) -> Duration {
        value.refresh_window().unwrap_or(self.advisory)
    }

    /// The window inside which a failed replacement of `value` is the
    /// answer: its own, else the lease's, never wider than its advisory
    /// window.
    fn mandatory_of(&self, value: &T) -> Duration {
        value
            .mandatory_window()
            .unwrap_or(self.mandatory)
            .min(self.advisory_of(value))
    }

    /// Whether [`Self::get`] at `now` answers from a hold - nothing found,
    /// or a failure inside its pause with nothing usable to answer instead
    /// - without obtaining: what a holder whose sources can change under it
    ///   asks before deciding the hold no longer applies.
    pub fn is_holding(&self, now: SystemTime) -> bool {
        let state = self.lock();
        let usable = state
            .held
            .as_ref()
            .is_some_and(|held| !lapses_within(held, now, self.mandatory_of(held)));
        let unsigned = state.held.is_none() && state.none_until.is_some_and(|until| now < until);
        let paused = state.failure.is_some() && state.retry_after.is_some_and(|at| now < at);
        unsigned || (paused && !usable)
    }

    /// The value held, without obtaining one.
    pub fn peek(&self) -> Option<T> {
        self.lock().held.clone()
    }

    /// Forget everything held and learned, so the next [`Self::get`]
    /// obtains again; an obtaining already running answers its own request
    /// and is held by nobody.
    pub fn invalidate(&self) {
        let mut state = self.lock();
        let emptied = State::empty(state.refreshing, state.generation.wrapping_add(1));
        *state = emptied;
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
/// The tools' own two habits are taken off here, where an expiry enters:
/// blanks around the text, and the trailing `UTC` - with or without a blank
/// before it - the AWS CLI's caches write for `Z`. What is left is read by
/// [`DateTime64::from_text`](crate::DateTime64::from_text), the crate's one
/// reader of datetime text, with a reading that names no zone taken as UTC,
/// the only zone any of the tools means. Anything it cannot read answers
/// `None`, so the value is treated as long-lived rather than lapsing at a
/// guessed instant.
pub fn instant(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let closed;
    let text = match text.strip_suffix("UTC") {
        Some(head) => {
            closed = format!("{}Z", head.trim_end());
            closed.as_str()
        }
        None => text,
    };
    let read = crate::DateTime64::from_text(text, crate::Timezone::UTC).ok()?;
    instant_of(read.count(), read.unit())
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
