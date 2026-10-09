//! What a medium knows of its origin, held between asks.
//!
//! One [`MediaCache`] per medium wrapper - Arrow IPC, Parquet, Avro, CSV,
//! text, XMLA, a workbook - holds one [`Entry`]: the whole root the origin
//! states, its row and column counts, and the medium's own object (a Parquet
//! footer, an Avro container's dimensions). An entry is served while the
//! handle is open, or while it is younger than the options'
//! [`cache_ttl`](crate::media::IORecordOptions::cache_ttl); `0`, the default,
//! is realtime, so a closed handle reads afresh on every ask. A write
//! refreshes the entry with what it published, and a verb that changes what
//! is stored without saying what - a merge, a byte write, a removal - drops
//! it. A container caches nothing.

use std::any::Any;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use smol_str::SmolStr;

use crate::{Error, Field, Result};

/// How long a medium's cached metadata is served off a closed handle, in
/// milliseconds; `0`, the default, is realtime: a closed handle re-reads on
/// every ask.
///
/// Outside every options value's identity, as the thread share one file
/// decodes on is: it changes when a change is seen, never what is, so two
/// options that differ only here compare, hash and order as equal, and the
/// hash every options value feeds is the same whatever it says.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheTtl(pub u64);

impl CacheTtl {
    /// No time-to-live: a closed handle reads its metadata afresh on every
    /// ask, and only an open one serves what it read.
    pub const REALTIME: Self = Self(0);

    /// The time-to-live in milliseconds.
    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }

    /// Whether this is [`Self::REALTIME`].
    #[must_use]
    pub const fn is_realtime(self) -> bool {
        self.0 == 0
    }

    /// Whether an entry stamped `at` is served at `now`: younger than the
    /// TTL, strictly, so an entry exactly as old as the TTL is read again;
    /// never under [`Self::REALTIME`].
    #[must_use]
    pub fn serves(self, at: Instant, now: Instant) -> bool {
        !self.is_realtime() && now.saturating_duration_since(at) < Duration::from_millis(self.0)
    }
}

impl PartialEq for CacheTtl {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for CacheTtl {}

impl PartialOrd for CacheTtl {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CacheTtl {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl std::hash::Hash for CacheTtl {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl From<u64> for CacheTtl {
    fn from(millis: u64) -> Self {
        Self(millis)
    }
}

/// A time-to-live spelled as text: a whole number of milliseconds, read by
/// the one integer grammar every count in the crate reads.
impl std::str::FromStr for CacheTtl {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        crate::integer::integer_from_text_as::<u64>(text)
            .map(Self)
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.cache_ttl"),
                reason: crate::text::expected_got(
                    "a whole number of milliseconds, 0 for realtime",
                    format_args!("{text:?}"),
                ),
            })
    }
}

/// What a medium knows of its origin at one instant.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// The whole root the origin holds, metadata whole; `None` where it
    /// states no shape (empty, a document with no schema).
    pub origin: Option<Field>,
    /// The rows the origin holds, where known.
    pub rows: Option<u64>,
    /// The columns of the origin's root, where known.
    pub columns: Option<usize>,
    /// The medium's own object: Parquet's `Arc<ParquetMetaData>`, Avro's
    /// dimensions, CSV's inferred `(CsvOptions, Field)`, the Excel
    /// `Workbook`; reached by `Arc::downcast`.
    pub state: Option<Arc<dyn Any + Send + Sync>>,
}

/// One medium's metadata cache: the entry, when it was stamped, whether the
/// handle is open, and a generation bumped by every invalidation.
///
/// The store is never read under the lock: [`Self::get_or_fill`] reads
/// between two short holds, so a fill that reads the medium again cannot
/// wait on itself.
#[derive(Debug, Default)]
pub struct MediaCache(Mutex<Inner>);

/// The state behind a [`MediaCache`]'s lock.
#[derive(Debug, Default)]
struct Inner {
    open: bool,
    generation: u64,
    at: Option<Instant>,
    entry: Option<Entry>,
}

impl MediaCache {
    /// An empty cache of a closed handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The explicit session starts: what is read from now on is served until
    /// [`Self::close`], whatever the TTL. An entry a closed handle held is
    /// dropped as the session starts, so the session serves what the store
    /// states from then on; opening an open handle keeps its entry.
    pub fn open(&self) {
        let mut inner = self.inner();
        if !inner.open {
            inner.open = true;
            inner.at = None;
            inner.entry = None;
        }
    }

    /// The session ends: the entry is dropped with it.
    pub fn close(&self) {
        let mut inner = self.inner();
        inner.open = false;
        inner.at = None;
        inner.entry = None;
    }

    /// Whether an explicit session is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner().open
    }

    /// How many invalidations the cache has seen.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner().generation
    }

    /// Whether what a read or a write learns is kept under `ttl`: while the
    /// session is open, or under a TTL above zero. A closed realtime handle
    /// keeps nothing, since nothing it stored would ever be served.
    #[must_use]
    pub fn keeps(&self, ttl: CacheTtl) -> bool {
        !ttl.is_realtime() || self.is_open()
    }

    /// Drop the entry and bump the generation, keeping the session: what a
    /// verb that changed the store without saying what calls.
    pub fn invalidate(&self) {
        let mut inner = self.inner();
        inner.at = None;
        inner.entry = None;
        inner.generation = inner.generation.wrapping_add(1);
    }

    /// The entry if served at `now` under `ttl`: while open, or while
    /// `ttl.serves(at, now)`.
    #[must_use]
    pub fn entry(&self, ttl: CacheTtl, now: Instant) -> Option<Entry> {
        let inner = self.inner();
        served(&inner, ttl, now).cloned()
    }

    /// Replace the entry, stamped `now`.
    ///
    /// Stored whatever the session: an entry is answered only by
    /// [`Self::entry`]'s rule, so what a closed handle holds is never served
    /// to a realtime ask, and it is replaced by the next fill and dropped by
    /// [`Self::invalidate`] and [`Self::close`].
    pub fn fill(&self, now: Instant, entry: Entry) {
        let mut inner = self.inner();
        inner.at = Some(now);
        inner.entry = Some(entry);
    }

    /// Edit the entry in place (an empty one if none), stamped `now`: what a
    /// write door calls with what it knows.
    pub fn update(&self, now: Instant, edit: impl FnOnce(&mut Entry)) {
        let mut inner = self.inner();
        edit(inner.entry.get_or_insert_with(Entry::default));
        inner.at = Some(now);
    }

    /// Add a fact read since to the entry served at `now` under `ttl`, its
    /// stamp kept: the entry's age still counts from the reading it was, so
    /// what it held before is served no longer than its TTL allows. Nothing
    /// where no entry is served, so a fact never stands for a shape nobody
    /// read.
    pub fn add(&self, ttl: CacheTtl, now: Instant, edit: impl FnOnce(&mut Entry)) {
        let mut inner = self.inner();
        let serves = served(&inner, ttl, now).is_some();
        if serves && let Some(entry) = inner.entry.as_mut() {
            edit(entry);
        }
    }

    /// Serve or fill: the entry under `ttl`, else `read()` filled and stamped
    /// `now` - only stored when the handle is open or `ttl` is not realtime
    /// (a realtime closed read stores nothing it could serve), and never
    /// over an invalidation that happened while `read` ran.
    ///
    /// # Errors
    ///
    /// Returns what `read` returns, and stores nothing then.
    pub fn get_or_fill(
        &self,
        ttl: CacheTtl,
        now: Instant,
        read: impl FnOnce() -> Result<Entry>,
    ) -> Result<Entry> {
        let generation = {
            let inner = self.inner();
            if let Some(entry) = served(&inner, ttl, now) {
                return Ok(entry.clone());
            }
            inner.generation
        };
        let entry = read()?;
        let mut inner = self.inner();
        if inner.generation == generation && (inner.open || !ttl.is_realtime()) {
            inner.at = Some(now);
            inner.entry = Some(entry.clone());
        }
        Ok(entry)
    }
}

/// The entry `inner` serves at `now` under `ttl`.
fn served(inner: &Inner, ttl: CacheTtl, now: Instant) -> Option<&Entry> {
    match (&inner.entry, inner.at) {
        (Some(entry), Some(at)) if inner.open || ttl.serves(at, now) => Some(entry),
        _ => None,
    }
}

#[cfg(feature = "internals")]
thread_local! {
    /// The instant an `internals` test installed on this thread.
    static CLOCK: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// The clock every door reads: `Instant::now()`, or the instant an
/// `internals` test installed on this thread.
pub(crate) fn now() -> Instant {
    #[cfg(feature = "internals")]
    {
        if let Some(now) = CLOCK.with(std::cell::Cell::get) {
            return now;
        }
    }
    Instant::now()
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/media/cache.rs` and the wrappers' suites pin and a
    //! caller cannot reach: the clock every cache door reads, set by hand.

    use std::time::Instant;

    /// Restores the clock a [`with_clock`] replaced, even when `run` panics.
    struct Restore(Option<Instant>);

    impl Drop for Restore {
        fn drop(&mut self) {
            super::CLOCK.with(|clock| clock.set(self.0));
        }
    }

    /// Run `run` with every cache door on this thread reading `now` as the
    /// time, the clock before restored after.
    pub fn with_clock<T>(now: Instant, run: impl FnOnce() -> T) -> T {
        let _restore = Restore(super::CLOCK.with(|clock| clock.replace(Some(now))));
        run()
    }
}
