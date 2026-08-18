//! The cached pages themselves: recency, expiry, pinning, and the budget.
//!
//! The table is a `HashMap` keyed by page index plus one monotonic counter.
//! At the default settings it holds 128 pages, so finding the
//! least-recently-used one with a linear scan costs less than maintaining a
//! second ordered structure beside the map - and it keeps the whole cache in
//! one allocation nobody has to keep in sync.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use super::BufferedOptions;

/// The map every cached page lives in.
type Pages = HashMap<u64, Page, PageHash>;

/// One cached page: the bytes it holds and when it was last touched.
///
/// `bytes` is shorter than the page size only at the end of the value, which
/// is what makes a short page also the record that the value ends there.
#[derive(Debug)]
struct Page {
    bytes: Vec<u8>,
    /// Last access, which is what the time to live is measured from.
    touched: Instant,
    /// Access order, so two pages touched inside one clock tick still compare.
    used: u64,
}

/// The page indexes a value's two ends occupy.
///
/// Both ends of a value are where discovery lives - magic bytes and media-type
/// sniffing at the head, a Parquet footer or an IPC schema block at the tail -
/// and they are re-read constantly, so they are exempt from eviction and from
/// expiry. The pin is derived from the *current* size on every operation, so
/// it follows the end rather than remembering where it used to be.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pinned {
    last: u64,
    /// Whether the value has any bytes, and therefore any page, to pin.
    held: bool,
}

impl Pinned {
    /// Return the pins a value of `size` bytes has.
    pub(crate) const fn for_size(size: u64, page_size: u64) -> Self {
        if size == 0 {
            return Self {
                last: 0,
                held: false,
            };
        }
        Self {
            last: (size - 1) / page_size,
            held: true,
        }
    }

    /// Return whether the page at `index` is pinned.
    pub(crate) const fn holds(self, index: u64) -> bool {
        self.held && (index == 0 || index == self.last)
    }
}

/// Hashing for dense page indexes.
///
/// The default hasher is `SipHash`, which exists to make a map holding
/// attacker-chosen *keys* safe; the keys here are page indexes of a bounded
/// table, where a collision costs one comparison and nothing else. A read is
/// the hot path this whole module exists to shorten, so it hashes with one
/// multiply and one rotation instead - measurably, in the `io_buffered`
/// benchmark's hit case.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PageHash;

impl BuildHasher for PageHash {
    type Hasher = PageHasher;

    fn build_hasher(&self) -> PageHasher {
        PageHasher(0)
    }
}

/// The state of one page-index hash.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PageHasher(u64);

impl Hasher for PageHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Unreached for a `u64` key, but a `Hasher` has to answer it.
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31);
    }
}

/// Every page currently cached, with its recency and its byte total.
#[derive(Debug, Default)]
pub(crate) struct PageTable {
    pages: Pages,
    /// Bytes the cached pages occupy together, pinned ones included.
    bytes: u64,
    /// Ticks once per access, so recency is a total order.
    clock: u64,
    /// The value's size as of the last operation that learned it.
    ///
    /// A read needs the size twice - to know where the value ends and to know
    /// which page is the pinned last one - and on a backend whose `size` is a
    /// metadata round trip, asking per read would leave the cache paying the
    /// very cost it exists to remove. So it is remembered here, refreshed
    /// whenever a read reaches the end of what the cache knows, and dropped
    /// with the pages.
    size: Option<u64>,
}

impl PageTable {
    /// Return the page at `index`, or nothing when it is absent or lapsed.
    ///
    /// A hit refreshes the page: the time to live is measured from the last
    /// access, so reading a page keeps it alive. A lapsed page is discarded
    /// here rather than by a sweeper thread - that is the whole of the lazy
    /// expiry.
    pub(crate) fn get(
        &mut self,
        index: u64,
        now: Instant,
        ttl: Duration,
        pinned: Pinned,
    ) -> Option<&[u8]> {
        let lapsed = {
            let page = self.pages.get(&index)?;
            !pinned.holds(index) && now.saturating_duration_since(page.touched) > ttl
        };
        if lapsed {
            self.remove(index);
            return None;
        }
        self.clock += 1;
        let clock = self.clock;
        let page = self.pages.get_mut(&index)?;
        page.touched = now;
        page.used = clock;
        Some(&page.bytes)
    }

    /// Store the page at `index`, expiring and evicting to stay in budget.
    ///
    /// The lapsed pages are swept here because a miss is already paying for an
    /// inner read; a hit stays a map lookup and a copy.
    pub(crate) fn insert(
        &mut self,
        index: u64,
        bytes: Vec<u8>,
        now: Instant,
        options: &BufferedOptions,
        pinned: Pinned,
    ) {
        self.sweep(now, options.ttl(), pinned);
        self.clock += 1;
        let page = Page {
            bytes,
            touched: now,
            used: self.clock,
        };
        self.bytes = self.bytes.saturating_add(page.bytes.len() as u64);
        if let Some(previous) = self.pages.insert(index, page) {
            self.bytes = self.bytes.saturating_sub(previous.bytes.len() as u64);
        }
        self.evict(options.max_bytes(), pinned);
    }

    /// Fold a write into the cached pages it changed.
    ///
    /// `previous_size` is the size before the write, because a write starting
    /// past the end zero-fills the gap: the changed range opens at the old end,
    /// not at the write's own offset. A page whose cached extent would have to
    /// *grow* is dropped instead of patched - that only happens at the end of
    /// the value, where a short page is the record that the value ended there.
    pub(crate) fn apply_write(
        &mut self,
        offset: u64,
        bytes: &[u8],
        previous_size: u64,
        current_size: u64,
        options: &BufferedOptions,
    ) {
        let page_size = options.page_size_u64();
        let start = offset.min(previous_size);
        // The changed range is bounded by where the value now ends, not only
        // by what was handed over: a write of *nothing* past the end still
        // grows the value and zero-fills the gap, which makes the page that
        // recorded the old end wrong however few bytes arrived.
        let written_end = offset.saturating_add(bytes.len() as u64);
        let end = if current_size > previous_size {
            written_end.max(current_size)
        } else {
            written_end
        };
        if start >= end {
            return;
        }
        let mut stale: Vec<u64> = Vec::new();
        for (index, page) in &mut self.pages {
            let page_start = options.page_start(*index);
            let region_end = page_start.saturating_add(page_size);
            if end <= page_start || start >= region_end {
                continue;
            }
            let cached_end = page_start.saturating_add(page.bytes.len() as u64);
            let change_end = end.min(region_end);
            if change_end > cached_end {
                stale.push(*index);
                continue;
            }
            let zero_from = start.max(page_start);
            let zero_to = offset.min(change_end);
            if zero_to > zero_from {
                page.bytes[slot(zero_from - page_start)..slot(zero_to - page_start)].fill(0);
            }
            let data_from = offset.max(page_start);
            // Only the bytes actually handed over are copied; the rest of the
            // changed range is growth, and a page reaching into that was
            // dropped above rather than patched.
            let data_to = change_end.min(written_end);
            if data_to > data_from {
                let from = slot(data_from - page_start);
                let to = slot(data_to - page_start);
                let source = slot(data_from - offset);
                page.bytes[from..to].copy_from_slice(&bytes[source..source + (to - from)]);
            }
        }
        for index in stale {
            self.remove(index);
        }
    }

    /// Drop every page that a resize at `limit` bytes could have changed.
    ///
    /// Only a *whole* page below the surviving prefix still holds exactly what
    /// the value holds there. A short page is the one at the end, and both
    /// shrinking and growing change what follows it - growing turns its
    /// recorded end into a false one - so it never survives a resize.
    pub(crate) fn retain_below(&mut self, limit: u64, options: &BufferedOptions) {
        let page_size = options.page_size();
        let mut freed = 0;
        self.pages.retain(|index, page| {
            let keep = page.bytes.len() == page_size
                && options
                    .page_start(*index)
                    .saturating_add(options.page_size_u64())
                    <= limit;
            if !keep {
                freed += page.bytes.len() as u64;
            }
            keep
        });
        self.bytes = self.bytes.saturating_sub(freed);
    }

    /// Drop every cached page, pinned ones included.
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.bytes = 0;
        self.size = None;
    }

    /// Return the size this cache last learned, if it has learned one.
    pub(crate) const fn known_size(&self) -> Option<u64> {
        self.size
    }

    /// Record the size an operation just learned or just produced.
    pub(crate) const fn set_size(&mut self, size: u64) {
        self.size = Some(size);
    }

    /// Return the bytes the cached pages occupy together.
    pub(crate) const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Return how many pages are cached.
    pub(crate) fn len(&self) -> usize {
        self.pages.len()
    }

    /// Return whether the page at `index` is cached, without touching it.
    pub(crate) fn contains(&self, index: u64) -> bool {
        self.pages.contains_key(&index)
    }

    /// Discard every page whose life ran out, keeping the pinned ones.
    fn sweep(&mut self, now: Instant, ttl: Duration, pinned: Pinned) {
        let mut freed = 0;
        self.pages.retain(|index, page| {
            let keep = pinned.holds(*index) || now.saturating_duration_since(page.touched) <= ttl;
            if !keep {
                freed += page.bytes.len() as u64;
            }
            keep
        });
        self.bytes = self.bytes.saturating_sub(freed);
    }

    /// Evict least-recently-used pages until the budget holds.
    ///
    /// Pinned pages are never candidates, so a budget with nothing else left
    /// to give stops here rather than spinning - which is exactly why
    /// [`BufferedOptions`] clamps the budget to at least two pages.
    fn evict(&mut self, max_bytes: u64, pinned: Pinned) {
        while self.bytes > max_bytes {
            let victim = self
                .pages
                .iter()
                .filter(|(index, _)| !pinned.holds(**index))
                .min_by_key(|(_, page)| page.used)
                .map(|(index, _)| *index);
            match victim {
                Some(index) => self.remove(index),
                None => break,
            }
        }
    }

    /// Drop one page, keeping the byte total honest.
    fn remove(&mut self, index: u64) {
        if let Some(page) = self.pages.remove(&index) {
            self.bytes = self.bytes.saturating_sub(page.bytes.len() as u64);
        }
    }
}

/// Narrow an in-page offset to an index, saturating rather than wrapping.
///
/// Every value passed here is already below one page size, so the saturation
/// is unreachable; it is written this way because a cast that could wrap has
/// no business in a cache that answers reads.
fn slot(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
