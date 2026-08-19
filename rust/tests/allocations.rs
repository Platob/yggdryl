//! What the text-record hot path allocates, counted rather than asserted.
//!
//! The claim the whole surface rests on is that a record is a *view* into a
//! bounded window: reading a million records must cost the same allocations as
//! reading a thousand, because none of them belongs to a record. A comment
//! saying so is not evidence, and a timing is not either - a per-record
//! allocation is cheap enough to hide inside I/O. So this counts them.
//!
//! The counting allocator is a global, and a program has exactly one, which is
//! why this is its own test target rather than a case in another file.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::io::{Buffer, IOBase};
use yggdryl::text::TextLineOptions;

/// A pass-through allocator that counts allocations while armed.
struct Counting;

/// Allocations since the counter was armed.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// Armed *per thread*, and const-initialized so reading it inside the allocator
// cannot itself allocate. Cargo runs the cases in this file concurrently, and a
// process-wide flag would charge one case for another's setup - which looks
// exactly like the per-record allocation these cases exist to catch.
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Held across a counted section, so only one thread is ever armed.
static COUNTING: Mutex<()> = Mutex::new(());

// SAFETY: every method forwards to `System`, which upholds the contract; the
// counter is an atomic and the flag is a const-initialized thread local, so
// neither adds aliasing nor re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: `layout` is forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer came from `System.alloc` with this same layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ARMED.get() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the pointer and layout came from this allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Count the allocations `work` performs, and return them with its answer.
fn counted<T>(work: impl FnOnce() -> T) -> (usize, T) {
    let guard = COUNTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ARMED.set(true);
    let answer = work();
    ARMED.set(false);
    let counted = ALLOCATIONS.load(Ordering::Relaxed);
    drop(guard);
    (counted, answer)
}

/// Refuse a count that scales with the record count.
///
/// The bound is deliberately loose - the window refills a few more times for a
/// bigger corpus, and that *is* proportional to the bytes - while still being
/// two orders of magnitude below what one allocation per record would produce.
/// It is the shape being pinned, not a golden number that churns.
fn flat(what: &str, small: usize, small_rows: usize, large: usize, large_rows: usize) {
    assert!(
        large * 100 < large_rows,
        "{what} must not allocate per record: {large} allocations over \
         {large_rows} records is more than one per hundred",
    );
    assert!(
        large < small * 4 + 32,
        "{what} allocations grew with the corpus: {small} over {small_rows} \
         records against {large} over {large_rows}",
    );
}

/// A corpus of `count` log records, built *before* anything is counted.
fn corpus(count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in 0..count {
        bytes.extend_from_slice(
            format!("2024-02-01T10:00:00 [INFO] row {index} of the corpus\n").as_bytes(),
        );
    }
    bytes
}

/// Drain every record, touching the accessors a real caller touches.
fn drain(handle: &yggdryl::text::Text<Buffer>) -> (usize, i64) {
    let mut records = handle.read_lines().expect("a reader");
    let mut count = 0;
    let mut hashes = 0_i64;
    while let Some(record) = records.next() {
        let record = record.expect("a record");
        // The checked view, the cached hash, and a capture: everything a row
        // is built from, and none of it may allocate.
        let text = record.text().expect("utf-8");
        assert!(!text.is_empty());
        hashes ^= record.hash().expect("a hash");
        count += 1;
    }
    (count, hashes)
}

#[test]
fn reading_records_allocates_the_same_for_a_thousand_and_for_ten_thousand() {
    let small = Buffer::from_bytes(corpus(1_000)).into_text();
    let large = Buffer::from_bytes(corpus(10_000)).into_text();

    // Warm anything lazily initialized once, uncounted, so the comparison is
    // between two drains rather than between a cold and a warm one.
    let _ = drain(&small);

    let (small_allocations, (small_rows, _)) = counted(|| drain(&small));
    let (large_allocations, (large_rows, _)) = counted(|| drain(&large));

    assert_eq!(small_rows, 1_000);
    assert_eq!(large_rows, 10_000);
    // Ten times the records, and the count barely moves: a record is a view
    // into the window, so nothing per record is allocated.
    flat(
        "reading",
        small_allocations,
        small_rows,
        large_allocations,
        large_rows,
    );
}

#[test]
fn matched_captures_and_a_pattern_add_no_per_record_allocation() {
    let options = TextLineOptions::with_pattern(r"^(?<stamp>\S+) \[(?<level>[A-Z]+)\]")
        .expect("a valid pattern");
    let small = Buffer::from_bytes(corpus(1_000)).into_text_with(options.clone());
    let large = Buffer::from_bytes(corpus(10_000)).into_text_with(options);

    let matched = |handle: &yggdryl::text::Text<Buffer>| {
        let mut records = handle.read_lines().expect("a reader");
        let mut levels = 0;
        while let Some(record) = records.next() {
            let record = record.expect("a record");
            // Captures are ranges into the same window; reading one is a slice.
            if record.capture("level").expect("a capture read") == Some("INFO") {
                levels += 1;
            }
            let _ = record.message().expect("a message");
        }
        levels
    };

    let _ = matched(&small);
    let (small_allocations, small_levels) = counted(|| matched(&small));
    let (large_allocations, large_levels) = counted(|| matched(&large));

    assert_eq!((small_levels, large_levels), (1_000, 10_000));
    // Matching allocates nothing per record either. `regex-lite` would build a
    // fresh `Captures` for every call and the span vector would be one per
    // match; both belong to the *reader* instead, which holds one of each and
    // rewrites them as records go by.
    flat(
        "matching",
        small_allocations,
        1_000,
        large_allocations,
        10_000,
    );
}

#[test]
fn writing_records_allocates_one_reused_buffer_however_many_there_are() {
    // Built before counting: the caller's own strings are the caller's cost.
    let thousand: Vec<String> = (0..1_000).map(|index| format!("row-{index}")).collect();
    let ten_thousand: Vec<String> = (0..10_000).map(|index| format!("row-{index}")).collect();

    let mut small = Buffer::new().into_text();
    let mut large = Buffer::new().into_text();
    // Warm the buffers to their final capacity first: growing the *sink* is a
    // cost of the bytes written, not of the records, and `Buffer` doubles.
    small.write_lines(&thousand).expect("a write");
    large.write_lines(&ten_thousand).expect("a write");

    let (small_allocations, ()) = counted(|| small.write_lines(&thousand).expect("a write"));
    let (large_allocations, ()) = counted(|| large.write_lines(&ten_thousand).expect("a write"));

    // Records accumulate in one reused buffer and flush in chunks.
    flat(
        "writing",
        small_allocations,
        1_000,
        large_allocations,
        10_000,
    );
}
