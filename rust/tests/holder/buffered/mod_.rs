//! `rust/src/holder/buffered/mod.rs`: the page-eviction clock a caller cannot set.
//!
//! `read_at` is the read that takes the instant to age a page from, which is
//! the only way to observe eviction and expiry without sleeping, and no caller
//! can name it, so it is reached through `yggdryl::internals`. Everything a
//! caller can observe lives in `rust/tests/holder/buffered.rs`, which counts
//! the same handle.

use std::time::{Duration, Instant};

use yggdryl::holder::buffered::{Buffered, BufferedOptions};
use yggdryl::internals::holder_buffered::read_at;

use crate::counting::Counting;

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
    read_at(&handle, page_size, &mut target, start).unwrap();
    read_at(&handle, 2 * page_size, &mut target, start).unwrap();

    // Six seconds on, page 1 is read again and page 2 is left alone.
    let midway = start + Duration::from_secs(6);
    let reads = handle.handle().reads();
    read_at(&handle, page_size, &mut target, midway).unwrap();
    assert_eq!(handle.handle().reads(), reads, "a hit inside the life");

    // Twelve seconds on, page 1 is six seconds old and page 2 is twelve. The
    // fetch of a third page is what sweeps the lapsed one out.
    let later = start + Duration::from_secs(12);
    read_at(&handle, page_size, &mut target, later).unwrap();
    assert_eq!(
        handle.handle().reads(),
        reads,
        "the repeatedly read page lives"
    );
    read_at(&handle, 3 * page_size, &mut target, later).unwrap();
    assert!(!handle.has_cached_page(2), "the untouched page died");

    // And reading it again is a miss, not a stale hit.
    let reads = handle.handle().reads();
    read_at(&handle, 2 * page_size, &mut target, later).unwrap();
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
    read_at(&handle, 16 * page_size - 8, &mut target, start).unwrap();
    read_at(&handle, 0, &mut target, start).unwrap();
    let after_ends = handle.handle().reads();

    // A scan of the middle, far past the budget and far past the life.
    let later = start + Duration::from_secs(60);
    for page in 1..15 {
        read_at(&handle, page * page_size, &mut target, later).unwrap();
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
    read_at(&handle, 0, &mut target, later).unwrap();
    read_at(&handle, 16 * page_size - 8, &mut target, later).unwrap();
    assert_eq!(handle.handle().reads(), reads);
    assert!(reads > after_ends, "the middle scan did reach the handle");
}
