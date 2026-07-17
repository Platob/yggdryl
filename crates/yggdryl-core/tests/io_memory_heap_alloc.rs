//! Deterministic allocation budgets for the in-heap [`Heap`](yggdryl_core::io::memory::Heap) source —
//! the fast, build-independent half of "validate both time and memory". Allocation *counts* do
//! not depend on the optimizer or the machine, so they can be asserted exactly and run in
//! milliseconds, guarding the zero-allocation typed accessors, the allocation-reusing
//! [`pread_into`](yggdryl_core::io::memory::IOBase::pread_into) transfer, and the capacity discipline
//! against regressions. (Throughput lives in the `heap` bench.)
//!
//! This file is its own test binary with its own counting global allocator, and holds a
//! **single** `#[test]` so nothing else allocates on another thread while a region is measured.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::memory::{Heap, IOBase};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Total allocations `op` makes over `iters` runs, after one warm-up run so any one-time
/// initialization stays outside the measured window.
fn allocs_over(iters: usize, mut op: impl FnMut()) -> usize {
    op();
    let before = ALLOCS.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    ALLOCS.load(Relaxed) - before
}

#[test]
fn allocation_budgets() {
    let iters = 1000;
    let data = Heap::from_slice(&[0u8; 64]);

    // Typed positioned accessors read into stack arrays — zero allocation.
    let typed_reads = allocs_over(iters, || {
        let _ = data.pread_byte(0).unwrap();
        let _ = data.pread_bit(3).unwrap();
        let _ = data.pread_i32(0).unwrap();
        let _ = data.pread_i64(0).unwrap();
    });
    assert_eq!(
        typed_reads, 0,
        "typed reads must be zero-alloc (got {typed_reads})"
    );

    // pread_into reuses the caller's buffer: once it is warmed to capacity, no further
    // allocation across an entire transfer loop.
    let mut scratch = Vec::with_capacity(64);
    let transfer = allocs_over(iters, || {
        let _ = data.pread_into(0, 32, &mut scratch);
    });
    assert_eq!(
        transfer, 0,
        "pread_into must reuse the buffer (got {transfer} allocs over {iters})"
    );

    // pread_vec, by contrast, allocates a fresh Vec every call — exactly one.
    let owning = allocs_over(iters, || {
        let _ = data.pread_vec(0, 32);
    });
    assert_eq!(
        owning, iters,
        "pread_vec must allocate exactly once per call (got {owning})"
    );

    // Overwriting within an already-sized (and pre-capacity) buffer never reallocates.
    let mut sink = Heap::from_slice(&[0u8; 16]);
    let overwrite = allocs_over(iters, || {
        sink.pwrite_byte(0, 1).unwrap();
        sink.pwrite_i32(1, -1).unwrap();
        sink.pwrite_i64(5, 1).unwrap();
        sink.pwrite_bit(120, true).unwrap();
    });
    assert_eq!(
        overwrite, 0,
        "in-place typed writes must not reallocate (got {overwrite})"
    );

    // A cursor round-trip that rewinds and overwrites the same region reallocates nothing.
    let mut cur = Heap::from_slice(&[0u8; 13]); // 1 + 4 + 8
    let cursor_roundtrip = allocs_over(iters, || {
        cur.rewind();
        cur.write_byte(0x7F).unwrap();
        cur.write_i32(-7).unwrap();
        cur.write_i64(1 << 40).unwrap();
        cur.rewind();
        let _ = cur.read_byte().unwrap();
        let _ = cur.read_i32().unwrap();
        let _ = cur.read_i64().unwrap();
    });
    assert_eq!(
        cursor_roundtrip, 0,
        "cursor round-trip over a sized buffer must not allocate (got {cursor_roundtrip})"
    );

    // with_capacity absorbs a build-up of writes without a single reallocation: filling to the
    // reserved capacity byte by byte stays at zero allocations.
    let build = allocs_over(iters, || {
        let mut h = Heap::with_capacity(256);
        for i in 0..256u64 {
            h.pwrite_byte(i, i as u8).unwrap();
        }
        // The only allocation per iteration is the initial `with_capacity` buffer.
    });
    assert_eq!(
        build, iters,
        "with_capacity fill must allocate exactly once (the reservation), got {build}"
    );

    // Slicing the heap owns a copy of the window — exactly one allocation.
    let sliced = allocs_over(iters, || {
        let _ = data.slice(8, 16).unwrap();
    });
    assert_eq!(
        sliced, iters,
        "slice must own its window in one allocation (got {sliced})"
    );

    // Bulk typed arrays stage through fixed STACK chunks — zero heap allocation, even across
    // multiple staging chunks (1000 elements > the 256-element chunk).
    let mut bulk_sink = Heap::with_capacity(8000);
    let bulk_values = vec![7i32; 1000];
    let mut bulk_back = vec![0i32; 1000];
    bulk_sink.pwrite_i32_array(0, &bulk_values).unwrap(); // pre-size once outside the window
    let bulk = allocs_over(iters, || {
        bulk_sink.pwrite_i32_array(0, &bulk_values).unwrap();
        bulk_sink.pread_i32_array(0, &mut bulk_back).unwrap();
    });
    assert_eq!(
        bulk, 0,
        "bulk i32 array ops must stage on the stack (got {bulk} allocs)"
    );

    // Repeated-value fills never materialize the full array — zero heap allocation once the
    // sink is sized.
    let mut fill_sink = Heap::with_capacity(8000);
    fill_sink.pwrite_i64_repeat(0, -1, 1000).unwrap();
    let fills = allocs_over(iters, || {
        fill_sink.pwrite_byte_repeat(0, 0xAB, 8000).unwrap();
        fill_sink.pwrite_i32_repeat(0, -7, 2000).unwrap();
        fill_sink.pwrite_i64_repeat(0, -1, 1000).unwrap();
    });
    assert_eq!(
        fills, 0,
        "repeat fills must never build the full array (got {fills} allocs)"
    );

    // A UTF-8 read owns exactly its String; the write allocates nothing beyond the (sized) sink.
    let mut text_sink = Heap::with_capacity(64);
    text_sink.pwrite_utf8(0, "hello wörld");
    let utf8 = allocs_over(iters, || {
        let _ = text_sink.pread_utf8(0, 12).unwrap();
    });
    assert_eq!(
        utf8, iters,
        "pread_utf8 must allocate exactly the returned String (got {utf8})"
    );
    let utf8_write = allocs_over(iters, || {
        let _ = text_sink.pwrite_utf8(0, "hello wörld");
    });
    assert_eq!(
        utf8_write, 0,
        "pwrite_utf8 into a sized sink must not allocate"
    );

    // Lightweight metadata: a heap initializes with an EMPTY headers map directly (an empty
    // map allocates nothing), so constructing a heap and reading untouched metadata is free.
    let lazy_headers = allocs_over(iters, || {
        let h = Heap::new();
        assert!(h.headers().is_empty());
        assert!(!h.headers().contains("anything"));
    });
    assert_eq!(
        lazy_headers, 0,
        "untouched heap metadata must allocate nothing (got {lazy_headers})"
    );

    // Auto-scaling appends amortize: appending 64 x 1 KiB chunks with NO reservation costs
    // only O(log n) reallocations (Vec doubling through the single-write append path) — far
    // fewer than one per chunk.
    let growth = allocs_over(1, || {
        let mut h = Heap::new();
        let chunk = [0u8; 1024];
        for _ in 0..64 {
            let end = h.byte_size();
            h.pwrite_byte_array(end, &chunk);
        }
    });
    assert!(
        growth <= 8,
        "64 chunked appends must cost O(log n) reallocations, got {growth}"
    );

    // A checked reservation that fails leaves the heap untouched and allocates nothing.
    let mut checked = Heap::with_capacity(64);
    let failed_reserve = allocs_over(iters, || {
        assert!(checked.try_reserve(u64::MAX).is_err());
    });
    assert_eq!(
        failed_reserve, 0,
        "a failed try_reserve must not allocate (got {failed_reserve})"
    );

    // Lazy-built address: `uri()` clones the once-parsed static — exactly the two small
    // string allocations of the clone ("mem" + "heap"), never a re-parse.
    let h = Heap::new();
    let lazy_uri = allocs_over(iters, || {
        let _ = h.uri();
    });
    assert_eq!(
        lazy_uri,
        2 * iters,
        "uri() must clone the cached mem://heap (2 small strings), not re-parse (got {lazy_uri})"
    );
}
