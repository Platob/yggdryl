//! What one record write did, in rows.

use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

/// The rows one record write read, wrote and skipped.
///
/// Every write door of [`IOMedia`](crate::IOMedia) answers one - the
/// overwrite, the append and the merge of every shape, and the generic
/// `write_*` beside them - so a caller knows what a write did without
/// reading the destination back. `read_rows` is what the write pulled from
/// its source, `written_rows` what reached the destination, and
/// `skipped_rows` what was read and not written: the rows the options'
/// `where` kept out, the part of the last batch a row or byte bound cut
/// off, and the rows an append to an Iceberg table stating
/// `identifier-field-ids` left out because the table held their key, or an
/// earlier row of the write brought it. A bound stops pulling, so what lies
/// past it was never read and is neither read nor skipped. A merge counts
/// every row it pulled as written, whether or not it changed the row it
/// matched.
///
/// A write that maps one row to one row answers
/// `read_rows == written_rows + skipped_rows`. A `select` that unnests
/// writes more rows than it read and skips none, so `skipped_rows` never
/// goes below zero. A write cut into several commits answers their sum.
///
/// ```
/// use yggdryl::IOResult;
///
/// let result = IOResult::new(10, 8);
/// assert_eq!(result.read_rows, 10);
/// assert_eq!(result.written_rows, 8);
/// assert_eq!(result.skipped_rows, 2);
/// assert_eq!(result.to_string(), "read 10 rows, wrote 8, skipped 2");
/// assert_eq!(result + IOResult::new(5, 5), IOResult::new(15, 13));
/// ```
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct IOResult {
    /// The rows the write pulled from its source.
    pub read_rows: u64,
    /// The rows that reached the destination.
    pub written_rows: u64,
    /// The rows read and not written.
    pub skipped_rows: u64,
}

impl IOResult {
    /// The result of a write that read `read_rows` and wrote `written_rows`,
    /// the rest of what it read skipped.
    #[must_use]
    pub const fn new(read_rows: u64, written_rows: u64) -> Self {
        Self {
            read_rows,
            written_rows,
            skipped_rows: read_rows.saturating_sub(written_rows),
        }
    }

    /// Whether the write read no row at all: its source was empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.read_rows == 0 && self.written_rows == 0
    }

    /// Return a deterministic hash of the three counts.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }
}

impl Add for IOResult {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            read_rows: self.read_rows.saturating_add(other.read_rows),
            written_rows: self.written_rows.saturating_add(other.written_rows),
            skipped_rows: self.skipped_rows.saturating_add(other.skipped_rows),
        }
    }
}

impl AddAssign for IOResult {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sum for IOResult {
    fn sum<I: Iterator<Item = Self>>(results: I) -> Self {
        results.fold(Self::default(), Add::add)
    }
}

impl fmt::Display for IOResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "read {} rows, wrote {}, skipped {}",
            self.read_rows, self.written_rows, self.skipped_rows
        )
    }
}
