//! What the page cache does, counted rather than assumed.
//!
//! Caching is only observable through the inner handle's traffic, so every
//! claim here is asserted against [`Counting`], a handle that mirrors a
//! [`Buffer`] and counts the `pread` and `pwrite` calls that reach it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::{Buffered, BufferedOptions};
use crate::generic::Holder;
use crate::io::{Buffer, IOBase};
use crate::{MediaType, MimeType, Url};

/// A handle that mirrors a [`Buffer`] and counts what reaches it.
///
/// It is the whole measuring instrument for this module: a cache is invisible
/// except through the reads it does *not* perform, so the tests assert inner
/// call counts rather than timings.
#[derive(Debug, Default)]
pub(crate) struct Counting {
    handle: Buffer,
    reads: AtomicUsize,
    writes: AtomicUsize,
    sizes: AtomicUsize,
}

impl Counting {
    /// Wrap bytes in a counting handle.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            handle: Buffer::from_bytes(bytes),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            sizes: AtomicUsize::new(0),
        }
    }

    /// Return how many `pread` calls have reached the buffer.
    pub(crate) fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    /// Return how many `pwrite` calls have reached the buffer.
    pub(crate) fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    /// Return how many `size` calls have reached the buffer.
    ///
    /// A `Buffer` answers this from a field, but the backends the cache
    /// exists for do not: an `arrowfs` handle answers it with one metadata
    /// call through the foreign filesystem's vtable, which over an object
    /// store is a round trip. Counting it is how the tests keep a cache hit
    /// from quietly costing one.
    pub(crate) fn sizes(&self) -> usize {
        self.sizes.load(Ordering::Relaxed)
    }
}

impl IOBase for Counting {
    crate::delegate_iobase!(handle: capacity, reserve, truncate, url,
        media_type, set_media_type, flush, parent, child_by, ls, kind);

    fn size(&self) -> u64 {
        self.sizes.fetch_add(1, Ordering::Relaxed);
        self.handle.size()
    }

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.handle.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
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

/// The bytes at `offset` the fixture holds, for comparing a read against.
fn expected(offset: usize, length: usize) -> Vec<u8> {
    (offset..offset + length).map(|index| index as u8).collect()
}

#[test]
fn a_second_read_of_the_same_range_reaches_nothing() {
    let handle = Buffered::new(counted(4 * PAGE), options(8));

    assert_eq!(handle.read_range(0, 16).unwrap(), expected(0, 16));
    let after_first = handle.handle().reads();
    assert!(after_first > 0, "the first read has to fetch the page");

    // Same range, and every range inside the same page, is memory only.
    assert_eq!(handle.read_range(0, 16).unwrap(), expected(0, 16));
    assert_eq!(handle.read_range(8, 40).unwrap(), expected(8, 40));
    assert_eq!(handle.handle().reads(), after_first);
    assert_eq!(handle.cached_pages(), 1);
}

#[test]
fn a_hit_asks_the_handle_for_nothing_at_all() {
    let handle = Buffered::new(counted(4 * PAGE), options(8));

    // The first read learns the size and fetches its page.
    assert_eq!(handle.read_range(0, 16).unwrap(), expected(0, 16));
    let reads = handle.handle().reads();
    let sizes = handle.handle().sizes();
    assert!(
        sizes > 0,
        "the first read has to learn where the value ends"
    );

    // Every later read the cache can serve whole reaches the handle for
    // nothing - not for bytes, and not for the size either. On a backend
    // whose `size` is a metadata round trip that is the difference between a
    // cache and a cache that still pays per read.
    //
    // The claim is about `pread`, the primitive: the derived helpers over it
    // - `read_range`, `read_all` - call `size` themselves, once per call,
    // exactly as they do over any other handle.
    let mut target = [0_u8; 40];
    for _ in 0..8 {
        assert_eq!(handle.pread(0, &mut target[..16]).unwrap(), 16);
        assert_eq!(handle.pread(8, &mut target).unwrap(), 40);
    }
    assert_eq!(&target[..8], expected(8, 8).as_slice());
    assert_eq!(handle.handle().reads(), reads);
    assert_eq!(handle.handle().sizes(), sizes);

    // A read that runs past what the cache knows re-asks once, because that
    // is where absence and growth are decided.
    assert_eq!(handle.pread(4 * PAGE as u64, &mut target).unwrap(), 0);
    assert_eq!(handle.handle().sizes(), sizes + 1);
}

#[test]
fn a_value_that_grew_is_seen_when_a_read_reaches_the_end() {
    let mut handle = Buffered::new(counted(PAGE), options(8));

    // Warm the cache, so the size the cache knows is the old one.
    assert_eq!(handle.read_all().unwrap().len(), PAGE);

    // Growth through the wrapper is known immediately, without asking.
    handle.pwrite(PAGE as u64, b"tail").unwrap();
    assert_eq!(handle.read_all().unwrap().len(), PAGE + 4);
    assert_eq!(
        handle.read_range(PAGE as u64, 4).unwrap(),
        b"tail",
        "the write is visible through the cache"
    );
}

#[test]
fn a_read_spanning_pages_caches_each_of_them() {
    let handle = Buffered::new(counted(8 * PAGE), options(16));

    // Unaligned on both ends, across four pages.
    let read = handle.read_range(PAGE as u64 - 3, 3 * PAGE + 6).unwrap();
    assert_eq!(read, expected(PAGE - 3, 3 * PAGE + 6));
    assert_eq!(handle.cached_pages(), 5);

    let fetched = handle.handle().reads();
    assert_eq!(handle.read_range(0, 5 * PAGE).unwrap().len(), 5 * PAGE);
    assert_eq!(
        handle.handle().reads(),
        fetched,
        "every page the span crossed is already held"
    );
}

#[test]
fn the_end_of_the_value_reads_short_and_past_it_reads_nothing() {
    // Ten bytes into the third page, so the last page is a short one.
    let size = 2 * PAGE + 10;
    let handle = Buffered::new(counted(size), options(8));

    let mut target = [0_u8; PAGE];
    assert_eq!(handle.pread(2 * PAGE as u64, &mut target).unwrap(), 10);
    assert_eq!(&target[..10], expected(2 * PAGE, 10).as_slice());

    // Reading entirely past the end is emptiness, and costs no inner read.
    let reads = handle.handle().reads();
    assert_eq!(handle.pread(size as u64, &mut target).unwrap(), 0);
    assert_eq!(handle.pread(1_000_000, &mut target).unwrap(), 0);
    assert_eq!(handle.handle().reads(), reads);

    // A read straddling the end is short, and repeating it stays free.
    assert_eq!(handle.pread(size as u64 - 4, &mut target).unwrap(), 4);
    let reads = handle.handle().reads();
    assert_eq!(handle.pread(size as u64 - 4, &mut target).unwrap(), 4);
    assert_eq!(handle.handle().reads(), reads);
}

#[test]
fn an_empty_handle_reads_nothing_and_caches_nothing() {
    let handle = Buffered::new(Counting::default(), options(4));

    let mut target = [0_u8; 8];
    assert_eq!(handle.pread(0, &mut target).unwrap(), 0);
    assert_eq!(handle.size(), 0);
    assert!(handle.read_all().unwrap().is_empty());
    assert_eq!(handle.cached_pages(), 0);
    assert_eq!(handle.handle().reads(), 0);
}

#[test]
fn a_write_is_seen_by_the_next_read() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(8));

    // Warm every page, then write across two of them.
    assert_eq!(handle.read_all().unwrap().len(), 4 * PAGE);
    handle.pwrite(PAGE as u64 - 2, b"ABCD").unwrap();

    assert_eq!(handle.read_range(PAGE as u64 - 2, 4).unwrap(), b"ABCD");
    assert_eq!(handle.handle().writes(), 1);
    // The patch keeps the surrounding bytes exactly as they were.
    assert_eq!(
        handle.read_range(PAGE as u64 - 4, 2).unwrap(),
        expected(PAGE - 4, 2)
    );
}

#[test]
fn a_write_extending_the_value_replaces_the_page_that_ended_it() {
    let mut handle = Buffered::new(counted(PAGE + 10), options(8));

    assert_eq!(handle.read_all().unwrap().len(), PAGE + 10);
    assert!(handle.has_cached_page(1));

    // The short page gains bytes, so it cannot survive as it was cached.
    handle.pwrite(PAGE as u64 + 10, b"tail").unwrap();
    assert!(!handle.has_cached_page(1));
    assert_eq!(handle.size(), PAGE as u64 + 14);
    assert_eq!(handle.read_range(PAGE as u64 + 10, 4).unwrap(), b"tail");
}

#[test]
fn a_write_that_only_grows_the_value_invalidates_the_page_that_ended_it() {
    let mut handle = Buffered::new(counted(10), options(4));
    assert_eq!(handle.read_all().unwrap(), expected(0, 10));
    assert!(handle.has_cached_page(0));

    // A write of nothing past the end still grows the value and zero-fills
    // the gap - `pwrite` says so - so the page that recorded where the value
    // ended is no longer the truth, however few bytes were handed over.
    handle.pwrite(20, b"").unwrap();
    assert_eq!(handle.size(), 20);

    let mut expected_bytes = expected(0, 10);
    expected_bytes.extend_from_slice(&[0; 10]);
    assert_eq!(handle.read_all().unwrap(), expected_bytes);
}

#[test]
fn a_write_past_the_end_zero_fills_what_the_cache_shows() {
    let mut handle = Buffered::new(counted(8), options(4));

    assert_eq!(handle.read_all().unwrap(), expected(0, 8));
    handle.pwrite(12, b"!").unwrap();

    assert_eq!(handle.size(), 13);
    assert_eq!(
        handle.read_all().unwrap(),
        b"\x00\x01\x02\x03\x04\x05\x06\x07\0\0\0\0!"
    );
}

#[test]
fn truncation_invalidates_both_ways() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(8));
    assert_eq!(handle.read_all().unwrap().len(), 4 * PAGE);
    assert_eq!(handle.cached_pages(), 4);

    // Shrinking drops everything at or past the new size.
    handle.truncate(2 * PAGE as u64).unwrap();
    assert_eq!(handle.cached_pages(), 2);
    assert!(handle.has_cached_page(0));
    assert!(!handle.has_cached_page(2));
    assert_eq!(handle.read_all().unwrap(), expected(0, 2 * PAGE));

    // Growing zero-fills, and the page that used to end the value goes with
    // it: what it recorded as the end is no longer the end.
    handle.truncate(2 * PAGE as u64 + 4).unwrap();
    assert!(handle.has_cached_page(0));
    assert!(!handle.has_cached_page(2));
    assert_eq!(handle.read_range(2 * PAGE as u64, 4).unwrap(), b"\0\0\0\0");
}

#[test]
fn eviction_takes_the_least_recently_read_page_first() {
    // Four pages of budget over a sixteen-page value.
    let handle = Buffered::new(counted(16 * PAGE), options(4));
    let page_size = PAGE as u64;

    // Pages 1 through 4, then page 1 again so page 2 is the oldest access.
    for page in 1..=4 {
        handle.read_range(page * page_size, 4).unwrap();
    }
    handle.read_range(page_size, 4).unwrap();

    // Page 5 needs room, and page 2 is what has gone longest untouched.
    handle.read_range(5 * page_size, 4).unwrap();
    assert!(handle.has_cached_page(1));
    assert!(!handle.has_cached_page(2));
    assert!(handle.cached_bytes() <= handle.options().max_bytes());

    // The budget holds however far the scan runs.
    for page in 6..16 {
        handle.read_range(page * page_size, 4).unwrap();
        assert!(handle.cached_bytes() <= handle.options().max_bytes());
    }
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

#[test]
fn the_pin_follows_the_end_the_value_grows() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(4));
    let page_size = PAGE as u64;
    let mut target = [0_u8; 8];

    // Page 3 is the last one, so it is pinned.
    handle.read_all().unwrap();
    assert_eq!(handle.cached_pages(), 4);

    // Growing by four pages moves the end; page 3 is now an ordinary page.
    handle.pwrite(8 * page_size - 1, b"z").unwrap();
    assert_eq!(handle.size(), 8 * page_size);

    // A scan that overruns the budget now evicts page 3, and pins page 7.
    for page in 4..8 {
        handle.pread(page * page_size, &mut target).unwrap();
    }
    handle.pread(5 * page_size, &mut target).unwrap();
    handle.pread(6 * page_size, &mut target).unwrap();
    assert!(handle.has_cached_page(0), "the header page is still pinned");
    assert!(handle.has_cached_page(7), "the new last page took the pin");
    assert!(
        !handle.has_cached_page(3),
        "the page that used to be last is evictable again"
    );
}

#[test]
fn truncating_below_the_footer_page_invalidates_it() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(8));

    handle.read_all().unwrap();
    assert!(handle.has_cached_page(3));

    handle.truncate(2 * PAGE as u64 + 8).unwrap();
    assert!(
        !handle.has_cached_page(3),
        "a pin is not immunity to a resize"
    );
    assert!(!handle.has_cached_page(2));
    assert_eq!(handle.read_all().unwrap(), expected(0, 2 * PAGE + 8));
}

#[test]
fn a_value_smaller_than_one_page_has_one_end() {
    let handle = Buffered::new(counted(10), options(4));

    handle.read_all().unwrap();
    assert_eq!(handle.cached_pages(), 1);

    // Head and tail are the same page; it is pinned once, not twice.
    let reads = handle.handle().reads();
    assert_eq!(handle.read_range(0, 10).unwrap(), expected(0, 10));
    assert_eq!(handle.read_range(6, 4).unwrap(), expected(6, 4));
    assert_eq!(handle.handle().reads(), reads);
    assert!(handle.cached_bytes() <= handle.options().max_bytes());
}

#[test]
fn an_exactly_two_page_value_is_all_pins() {
    let handle = Buffered::new(counted(2 * PAGE), options(2));
    let page_size = PAGE as u64;
    let mut target = [0_u8; 8];

    handle.pread(0, &mut target).unwrap();
    handle.pread(page_size, &mut target).unwrap();
    assert_eq!(handle.cached_pages(), 2);
    assert_eq!(handle.cached_bytes(), handle.options().max_bytes());

    // Nothing can be evicted, and nothing needs to be: both ends are held.
    let reads = handle.handle().reads();
    handle.pread(0, &mut target).unwrap();
    handle.pread(page_size, &mut target).unwrap();
    assert_eq!(handle.handle().reads(), reads);
}

#[test]
fn a_budget_below_two_pages_is_clamped() {
    let options = BufferedOptions::default()
        .with_page_size(PAGE)
        .with_max_bytes(1);
    assert_eq!(options.max_bytes(), 2 * PAGE as u64);

    // Raising the page size re-applies the clamp to a budget set earlier.
    let grown = options
        .with_max_bytes(4 * PAGE as u64)
        .with_page_size(4 * PAGE);
    assert_eq!(grown.page_size(), 4 * PAGE);
    assert_eq!(grown.max_bytes(), 8 * PAGE as u64);

    // A page size that is not a power of two is rounded up to one.
    assert_eq!(
        BufferedOptions::default().with_page_size(100).page_size(),
        128
    );
    assert_eq!(BufferedOptions::default().with_page_size(0).page_size(), 64);
}

#[test]
fn buffering_a_buffered_handle_re_wraps_it() {
    let once = counted(PAGE).buffered(options(4));
    once.read_range(0, 4).unwrap();

    // The inherent method wins, so this is `Buffered<Counting>` again.
    let twice: Buffered<Counting> = once.buffered(options(8).with_page_size(2 * PAGE));
    assert_eq!(twice.options().page_size(), 2 * PAGE);
    assert_eq!(
        twice.cached_pages(),
        0,
        "a re-wrap starts with a fresh cache"
    );
    assert_eq!(twice.read_range(0, 4).unwrap(), expected(0, 4));

    // And a holder never nests either.
    let holder = Holder::buffer(Buffer::from_bytes(vec![1_u8; 32])).buffered(options(4));
    let holder = holder.buffered(options(4));
    match &holder {
        Holder::Buffered(inner) => assert!(
            matches!(inner.handle(), Holder::Buffer(_)),
            "the held io is the buffer, not a second cache"
        ),
        other => panic!("expected a buffered holder, got {other:?}"),
    }
    assert_eq!(holder.read_all().unwrap(), vec![1_u8; 32]);
}

#[test]
fn everything_but_the_reads_is_the_wrapped_handle() {
    let inner = Buffer::from_bytes(b"symbol,price\n".to_vec())
        .with_media_type(MediaType::from(MimeType::CSV));
    let url = inner.url().cloned();
    let mut handle = inner.buffered(options(4));

    assert_eq!(handle.media_type().base(), &MimeType::CSV);
    assert_eq!(handle.url(), url.as_ref());
    assert_eq!(handle.kind(), crate::IOKind::Memory);
    assert_eq!(handle.size(), 13);
    assert_eq!(handle.capacity(), handle.handle().capacity());
    assert!(handle.parent().is_none());
    assert!(handle.ls(true, true).unwrap().is_empty());

    handle.set_media_type(MediaType::from(MimeType::JSON));
    assert_eq!(handle.handle().media_type().base(), &MimeType::JSON);

    handle.reserve(4_096).unwrap();
    assert!(handle.capacity() >= 4_096);
    assert_eq!(handle.size(), 13);
}

#[test]
fn closing_drops_every_page_and_the_handle_keeps_working() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(8));

    handle.read_all().unwrap();
    assert_eq!(handle.cached_pages(), 4);
    let reads = handle.handle().reads();

    handle.close().unwrap();
    assert_eq!(handle.cached_pages(), 0);
    assert!(!handle.has_cached_page(0), "the pinned pages go too");

    // Still a working handle: the next read simply fetches again.
    assert_eq!(handle.read_range(0, 4).unwrap(), expected(0, 4));
    assert!(handle.handle().reads() > reads);
}

#[test]
fn reaching_the_inner_handle_mutably_drops_the_cache() {
    let mut handle = Buffered::new(counted(4 * PAGE), options(8));
    handle.read_all().unwrap();
    assert_eq!(handle.cached_pages(), 4);

    // Bytes written straight to the inner handle are invisible here, so the
    // borrow drops the cache rather than leaving it to go stale.
    handle.handle_mut().pwrite(0, b"XYZ").unwrap();
    assert_eq!(handle.cached_pages(), 0);
    assert_eq!(handle.read_range(0, 3).unwrap(), b"XYZ");

    // And the explicit spelling of the same thing.
    handle.read_all().unwrap();
    handle.clear_cache();
    assert_eq!(handle.cached_pages(), 0);
}

#[test]
fn a_url_named_handle_keeps_its_own_identity() {
    let inner =
        Buffer::new().with_media_type(Url::from_str("file:///trades.arrows").unwrap().media_type());
    let handle = inner.buffered(BufferedOptions::default());
    assert_eq!(handle.media_type().base(), &MimeType::ARROW_STREAM);
}
