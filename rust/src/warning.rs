//! Deduplicated warnings: what a data door that passes over a value it
//! cannot read says, once.
//!
//! A FIX parse, a market data read and a lifecycle walk never fail on the
//! data they are handed: a fact that does not matter takes its default, an
//! item that cannot stand is excluded, and either is said as a warning
//! naming what was wrong, where, and what was done instead. A stream says
//! the same thing once per row it happens on, so each warning is keyed by
//! where it is raised, what went wrong and the column, tag or kind it is
//! about - never by the value or the row - and logged the first time that
//! key is seen, then again at each tenfold count, with the count.
//!
//! The table is process-wide and bounded at [`CAPACITY`] keys, the bound a
//! stream of distinct subjects cannot grow past: past it, one line says so
//! and new keys are counted in total only. A key already seen costs one
//! hash and one lock, and builds no text.

use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::{Mutex, OnceLock};

use crate::xxhash::Xxh3;

/// The most distinct warnings the process remembers. Bounded so a stream
/// whose subjects never repeat cannot grow memory without limit.
pub(crate) const CAPACITY: usize = 4096;

/// What the table holds: each key's count, and how many warnings arrived
/// once it was full.
#[derive(Default)]
struct Seen {
    counts: HashMap<u64, u64>,
    overflow: u64,
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Mutex::default)
}

/// The key one warning is deduplicated under.
fn key(site: &str, what: &str, subject: &str) -> u64 {
    let mut state = Xxh3::new();
    for part in [site, what, subject] {
        state.write(part.as_bytes());
        state.write(&[0]);
    }
    state.finish()
}

/// Counts one occurrence, answering its count - `0` once the table is full
/// and the key is not in it.
fn count(site: &str, what: &str, subject: &str) -> u64 {
    let key = key(site, what, subject);
    let mut seen = seen()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(count) = seen.counts.get_mut(&key) {
        *count += 1;
        return *count;
    }
    if seen.counts.len() >= CAPACITY {
        seen.overflow += 1;
        let overflow = seen.overflow;
        if overflow == 1 || is_tenfold(overflow) {
            log::warn!(
                target: "yggdryl::warning",
                "{CAPACITY} distinct warnings were logged; {overflow} of other kinds were not"
            );
        }
        return 0;
    }
    seen.counts.insert(key, 1);
    1
}

/// Whether `count` is a tenfold one past the first: `10`, `100`, `1000`...
fn is_tenfold(mut count: u64) -> bool {
    if count < 10 {
        return false;
    }
    while count % 10 == 0 {
        count /= 10;
    }
    count == 1
}

/// Warns once about `subject` - the column, tag, field or kind a warning is
/// about - under `what` went wrong at `site`: `detail` is built and logged
/// the first time, naming the value, the location and what was done
/// instead, and the count is logged at each tenfold occurrence after it.
///
/// `what` and `subject` are the key and must not hold the value or the row,
/// or every row would be its own warning.
pub(crate) fn warn(
    site: &'static str,
    what: &'static str,
    subject: &str,
    detail: impl FnOnce() -> String,
) {
    match count(site, what, subject) {
        1 => log::warn!(
            target: site,
            "{what} ({subject}): {}; later occurrences are counted rather than repeated",
            detail()
        ),
        count if is_tenfold(count) => {
            log::warn!(target: site, "{what} ({subject}): seen {count} times");
        }
        _ => {}
    }
}

/// [`warn`] at the calling module: `warned!(what, subject, "format", args..)`,
/// the format built only the first time the key is seen.
macro_rules! warned {
    ($what:expr, $subject:expr, $($format:tt)+) => {
        $crate::warning::warn(module_path!(), $what, $subject, || format!($($format)+))
    };
}
pub(crate) use warned;

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/warning.rs` pins and a caller cannot reach.

    /// Warns once under `site`, as a door does.
    pub fn warn(site: &'static str, what: &'static str, subject: &str, detail: &str) {
        super::warn(site, what, subject, || detail.to_owned());
    }

    /// How many times the warning `what` about `subject` was raised at
    /// `site` - a module path, such as `yggdryl::graph::book` - `0` for
    /// one never raised.
    #[must_use]
    pub fn count(site: &str, what: &str, subject: &str) -> u64 {
        let key = super::key(site, what, subject);
        super::seen()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .counts
            .get(&key)
            .copied()
            .unwrap_or(0)
    }

    /// Whether `count` is one a repeated warning is logged at again.
    #[must_use]
    pub fn is_tenfold(count: u64) -> bool {
        super::is_tenfold(count)
    }
}
