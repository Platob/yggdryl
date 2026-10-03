//! Repeated records counted by hash: the one deduplication every logger,
//! every data warning and every binding filter counts through.

use std::fmt::{self, Write as _};
use std::hash::Hasher;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::Record;
use crate::xxhash::Xxh3;

/// The most distinct keys one table counts.
const CAPACITY: usize = 4096;

/// The slots a table probes: twice its capacity, so a probe stays short
/// however full the table is allowed to get.
const SLOTS: usize = CAPACITY * 2;

/// What one occurrence of a key is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Repeat {
    /// The key's first occurrence: said in full.
    First,
    /// A tenfold occurrence - the 10th, 100th, 1000th: said again with its
    /// count.
    Tenfold(u64),
    /// Any other occurrence: said nothing of.
    Repeated,
    /// A key the table had no room left to count: a deduplicating logger
    /// says it in full, uncounted; the crate's data warnings count such
    /// keys together and say how many there were.
    Untracked,
}

impl Repeat {
    /// Whether the occurrence is said at all.
    pub const fn is_said(self) -> bool {
        !matches!(self, Self::Repeated)
    }
}

/// A table counting how often each key occurred - a record's
/// [`stable_hash`](Record::stable_hash), or any key a caller hashes -
/// bounded at [`Self::CAPACITY`] keys.
///
/// A count is two atomic operations on a fixed array of slots, found by the
/// key's own bits: no lock, no allocation past the slots' one allocation on
/// the first count, and two threads counting one key see two different
/// counts, so exactly one of them sees the first. A full table counts no new
/// key - [`Repeat::Untracked`] - so a stream of messages that never repeat
/// cannot grow memory, and a key already counted keeps counting.
///
/// ```
/// use yggdryl::logging::{Level, Record, Repeat, Repeats};
///
/// let repeats = Repeats::new();
/// let record = Record::new("trades.feed", Level::WARNING, &"late fill");
/// assert_eq!(repeats.count_record(&record), Repeat::First);
/// for _ in 2..10 {
///     assert_eq!(repeats.count_record(&record), Repeat::Repeated);
/// }
/// assert_eq!(repeats.count_record(&record), Repeat::Tenfold(10));
/// assert_eq!(repeats.seen(record.stable_hash()), 10);
/// ```
#[derive(Default)]
pub struct Repeats {
    slots: OnceLock<Box<[Slot]>>,
    held: AtomicUsize,
}

impl fmt::Debug for Repeats {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Repeats")
            .field("len", &self.len())
            .finish()
    }
}

/// One key and its count; a key of `0` marks the slot empty.
#[derive(Default)]
struct Slot {
    key: AtomicU64,
    count: AtomicU64,
}

impl Repeats {
    /// The most distinct keys a table counts.
    pub const CAPACITY: usize = CAPACITY;

    /// An empty table; its slots are allocated on the first count.
    pub const fn new() -> Self {
        Self {
            slots: OnceLock::new(),
            held: AtomicUsize::new(0),
        }
    }

    /// Counts one occurrence of `key`.
    pub fn count(&self, key: u64) -> Repeat {
        match self.slot(key, true) {
            Some(slot) => match slot.count.fetch_add(1, Ordering::Relaxed) + 1 {
                1 => Repeat::First,
                count if is_tenfold(count) => Repeat::Tenfold(count),
                _ => Repeat::Repeated,
            },
            None => Repeat::Untracked,
        }
    }

    /// Counts one occurrence of `record`: its logger, level and message.
    pub fn count_record(&self, record: &Record<'_>) -> Repeat {
        self.count(record.stable_hash())
    }

    /// How many times `key` was counted; `0` for one never counted.
    pub fn seen(&self, key: u64) -> u64 {
        self.slot(key, false)
            .map_or(0, |slot| slot.count.load(Ordering::Relaxed))
    }

    /// How many distinct keys are counted.
    pub fn len(&self) -> usize {
        self.held.load(Ordering::Relaxed)
    }

    /// Whether no key is counted yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The slot holding `key`, claimed for it when `claim` and the table
    /// has room; linear probing from the key's own low bits.
    // `fetch_update` is the spelling the declared MSRV, Rust 1.94, knows;
    // its rename `try_update` came later.
    #[allow(deprecated)]
    fn slot(&self, key: u64, claim: bool) -> Option<&Slot> {
        // Zero marks an empty slot, so the one key that is zero is counted
        // as one.
        let key = key.max(1);
        let slots = if claim {
            self.slots
                .get_or_init(|| (0..SLOTS).map(|_| Slot::default()).collect())
        } else {
            self.slots.get()?
        };
        #[allow(clippy::cast_possible_truncation)]
        let mut at = key as usize & (SLOTS - 1);
        for _ in 0..SLOTS {
            let slot = &slots[at];
            match slot.key.load(Ordering::Acquire) {
                held if held == key => return Some(slot),
                0 if !claim => return None,
                0 => {
                    // Room is reserved before the claim, so racing claimers
                    // never take the table past its capacity.
                    let reserved =
                        self.held
                            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                                (held < CAPACITY).then_some(held + 1)
                            });
                    if reserved.is_err() {
                        // Full - unless the last room went to this very key.
                        return (slot.key.load(Ordering::Acquire) == key).then_some(slot);
                    }
                    match slot
                        .key
                        .compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire)
                    {
                        Ok(_) => return Some(slot),
                        Err(held) => {
                            self.held.fetch_sub(1, Ordering::AcqRel);
                            if held == key {
                                return Some(slot);
                            }
                        }
                    }
                }
                _ => {}
            }
            at = (at + 1) & (SLOTS - 1);
        }
        None
    }
}

/// Whether `count` is a tenfold one past the first: `10`, `100`, `1000`...
pub(crate) const fn is_tenfold(mut count: u64) -> bool {
    if count < 10 {
        return false;
    }
    while count.is_multiple_of(10) {
        count /= 10;
    }
    count == 1
}

/// A repeated message said again with its count: `late fill (seen 10
/// times)`.
#[derive(Clone, Copy)]
pub struct Counted<'a> {
    count: u64,
    message: &'a dyn fmt::Display,
}

impl<'a> Counted<'a> {
    /// `message`, said for the `count`th time.
    pub fn new(count: u64, message: &'a dyn fmt::Display) -> Self {
        Self { count, message }
    }
}

impl fmt::Display for Counted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (seen {} times)", self.message, self.count)
    }
}

impl fmt::Debug for Counted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_string())
    }
}

/// A hasher a message renders into, so its key is computed without the
/// message ever being held as text.
struct Feed(Xxh3);

impl fmt::Write for Feed {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}

impl Record<'_> {
    /// XXH3-64 of the logger's name, the level and the rendered message,
    /// each framed so that no two of them run together: the key a repeated
    /// record is counted by. The message is rendered into the hash, never
    /// into text.
    pub fn stable_hash(&self) -> u64 {
        let mut feed = Feed(Xxh3::new());
        feed.0.write(self.name().as_bytes());
        feed.0.write(&[0, self.level().get(), 0]);
        let _ = write!(feed, "{}", self.message());
        feed.0.finish()
    }
}

/// The table the tree's deduplicating loggers count records in.
pub(super) static RECORDS: Repeats = Repeats::new();
