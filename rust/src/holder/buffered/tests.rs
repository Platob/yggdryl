//! The page-eviction clock an integration test cannot reach.
//!
//! `read_at` is the crate-private read that takes the instant to age a page
//! from, which is the only way to observe eviction and expiry without
//! sleeping. Everything a caller can observe lives in
//! `tests/holder/buffered.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::{Buffered, BufferedOptions};
use crate::holder::Buffer;
use crate::{IOBase, Result};

/// A handle that mirrors a [`Buffer`] and counts the reads that reach it.
///
/// A cache hit is only observable as a read that did *not* happen, so these
/// two need a counter as well as the clock. `tests/holder/buffered.rs` has its
/// own copy for the same reason: an integration test cannot see a
/// `#[cfg(test)]` item, and making this one public would put a measuring
/// instrument in the crate's API.
#[derive(Debug, Default)]
struct Counting {
    handle: Buffer,
    reads: AtomicUsize,
}

impl Counting {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            handle: Buffer::from_bytes(bytes),
            reads: AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl crate::IOMedia for Counting {
    crate::impl_default_iomedia!();
}

impl IOBase for Counting {
    crate::delegate_iobase!(handle: capacity, reserve, truncate, url,
        media_type, set_media_type, flush, parent, child_by_path, ls, kind);

    fn size(&self) -> u64 {
        self.handle.size()
    }

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.handle.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.handle.pwrite(offset, bytes)
    }
}

/// A page size small enough that a modest fixture spans many pages.
const PAGE: usize = 64;

/// Options over `PAGE`-sized pages with room for `pages` of them.
fn options(pages: u64) -> BufferedOptions {
    BufferedOptions::default()
        .with_page_size(PAGE)
        .with_max_bytes(pages * PAGE as u64)
}

/// A counting handle holding `size` bytes whose value names its own offset.
fn counted(size: usize) -> Counting {
    Counting::from_bytes((0..size).map(|index| index as u8).collect())
}

#[test]
fn a_page_lives_from_its_last_access() {
    let handle = Buffered::new(
        counted(16 * PAGE),
        options(8).with_ttl(Duration::from_secs(10)),
    );
    let page_size = PAGE as u64;
    let start = Instant::now();
    let mut target = [0_u8; 4];

    // Two pages read at the same instant.
    handle.read_at(page_size, &mut target, start).unwrap();
    handle.read_at(2 * page_size, &mut target, start).unwrap();

    // Six seconds on, page 1 is read again and page 2 is left alone.
    let midway = start + Duration::from_secs(6);
    let reads = handle.handle().reads();
    handle.read_at(page_size, &mut target, midway).unwrap();
    assert_eq!(handle.handle().reads(), reads, "a hit inside the life");

    // Twelve seconds on, page 1 is six seconds old and page 2 is twelve. The
    // fetch of a third page is what sweeps the lapsed one out.
    let later = start + Duration::from_secs(12);
    handle.read_at(page_size, &mut target, later).unwrap();
    assert_eq!(
        handle.handle().reads(),
        reads,
        "the repeatedly read page lives"
    );
    handle.read_at(3 * page_size, &mut target, later).unwrap();
    assert!(!handle.has_cached_page(2), "the untouched page died");

    // And reading it again is a miss, not a stale hit.
    let reads = handle.handle().reads();
    handle.read_at(2 * page_size, &mut target, later).unwrap();
    assert_eq!(handle.handle().reads(), reads + 1);
}

#[test]
fn the_two_ends_outlive_eviction_and_expiry() {
    let handle = Buffered::new(
        counted(16 * PAGE),
        options(4).with_ttl(Duration::from_secs(10)),
    );
    let page_size = PAGE as u64;
    let start = Instant::now();
    let mut target = [0_u8; 8];

    // Footer first, then the header: the shape a container is opened with.
    handle
        .read_at(16 * page_size - 8, &mut target, start)
        .unwrap();
    handle.read_at(0, &mut target, start).unwrap();
    let after_ends = handle.handle().reads();

    // A scan of the middle, far past the budget and far past the life.
    let later = start + Duration::from_secs(60);
    for page in 1..15 {
        handle
            .read_at(page * page_size, &mut target, later)
            .unwrap();
    }
    assert!(handle.cached_bytes() <= handle.options().max_bytes());
    assert!(handle.has_cached_page(0), "the header page is pinned");
    assert!(handle.has_cached_page(15), "the footer page is pinned");
    assert!(
        !handle.has_cached_page(7),
        "a middle page is neither pinned nor recent"
    );

    // Re-reading either end costs nothing, however long ago it was read.
    let reads = handle.handle().reads();
    handle.read_at(0, &mut target, later).unwrap();
    handle
        .read_at(16 * page_size - 8, &mut target, later)
        .unwrap();
    assert_eq!(handle.handle().reads(), reads);
    assert!(reads > after_ends, "the middle scan did reach the handle");
}
