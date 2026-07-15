//! Deterministic allocation budgets for [`Bytes`] — the memory half of "validate time and
//! memory". Allocation *counts* are optimizer- and machine-independent, so they assert the
//! zero-copy / copy-on-write design directly: a positioned read and a slice touch **no**
//! heap, an in-place write reuses the payload allocation, and a write to a *shared* slice
//! costs exactly one extra allocation (the copy-on-write payload) — proving writes copy only
//! when the allocation is actually shared.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]` so
//! nothing else allocates on another thread while a region is measured.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::{Bytes, IOBase, IOCursor, IOSlice};

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

/// Total allocations `op` makes over `iters` runs, after one warm-up run.
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
    let data = Bytes::from_vec(vec![7u8; 4096]);
    let payload = [1u8; 1024];

    // A positioned read copies into the caller's buffer — no heap at all.
    let mut scratch = [0u8; 1024];
    let pread = allocs_over(iters, || {
        let _ = data.pread(0, &mut scratch);
    });
    assert_eq!(pread, 0, "pread must be zero-copy (got {pread})");

    // A slice shares the parent's Arc allocation — an atomic refcount bump, no heap.
    let slice = allocs_over(iters, || {
        let _ = data.slice(512, 1024).unwrap();
    });
    assert_eq!(slice, 0, "slice must be zero-copy (got {slice})");

    // An in-place write to a uniquely-owned buffer reuses the payload allocation; the only
    // per-write allocation is the small Arc control block (independent of payload size).
    let mut owned = Bytes::from_vec(vec![0u8; 4096]);
    let inplace = allocs_over(iters, || {
        let _ = owned.pwrite(0, &payload);
    });

    // A write to a slice that still shares its parent's allocation copies-on-write: exactly
    // ONE allocation more than the in-place case (the copied payload). Re-slicing each
    // iteration re-establishes the sharing (the slice itself allocates nothing).
    let shared_parent = Bytes::from_vec(vec![0u8; 4096]);
    let cow = allocs_over(iters, || {
        let mut window = shared_parent.slice(0, 4096).unwrap();
        window.pwrite(0, &payload);
    });
    assert_eq!(
        cow,
        inplace + iters,
        "a copy-on-write write must cost exactly one more allocation than in-place \
         (cow {cow} vs inplace {inplace} + {iters})"
    );

    // The owning reads allocate exactly their one output buffer.
    let pread_vec = allocs_over(iters, || {
        let _ = data.pread_vec(0, 1024);
    });
    assert_eq!(
        pread_vec, iters,
        "pread_vec allocates one output (got {pread_vec})"
    );

    let mut reader = data.clone();
    let read_to_end = allocs_over(iters, || {
        reader.rewind();
        let _ = reader.read_to_end();
    });
    assert_eq!(
        read_to_end, iters,
        "read_to_end allocates one output (got {read_to_end})"
    );
}
