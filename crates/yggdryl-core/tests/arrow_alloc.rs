//! Deterministic allocation budget for the Arrow leaf bridge
//! ([`column_to_arrow`](yggdryl_core::arrow::column_to_arrow), feature `arrow`) — the fast half of
//! "validate both time and memory". The invariant guarded: a leaf `Column -> Arrow array` conversion
//! allocates a **small constant number of times, independent of the row count** — one bulk buffer
//! copy per column buffer plus fixed Arrow bookkeeping, *never* one allocation per element. Comparing
//! a 1 000-row column against a 100 000-row column and asserting the **same** per-op allocation count
//! is the strongest form of that proof.
//!
//! This file is its own test binary with its own counting global allocator, and holds a **single**
//! `#[test]` so nothing else allocates on another thread while a region is measured.
#![cfg(feature = "arrow")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::arrow::column_to_arrow;
use yggdryl_core::typed::fixedbyte::Int64;
use yggdryl_core::typed::{Column, FixedSerie};

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
fn column_to_arrow_copies_once_per_buffer() {
    let iters = 1000;

    // A non-null `Int64` column: to-Arrow reinterprets its 8-byte-per-element data buffer into one
    // owning Arrow `Buffer` (a single bulk copy — the entry point borrows the `&Column`, so the
    // owning `Heap` cannot be moved into a zero-copy `Buffer::from_vec`), wraps it in a `ScalarBuffer`
    // (no copy), builds the `Int64Array` (no copy), and `Arc`s it. That is a small constant number of
    // allocations, the same whether the column is 1 000 or 100 000 rows long.
    let small = Column::from(FixedSerie::<Int64>::from_values(
        &(0..1_000i64).collect::<Vec<_>>(),
    ));
    let big = Column::from(FixedSerie::<Int64>::from_values(
        &(0..100_000i64).collect::<Vec<_>>(),
    ));

    let small_allocs = allocs_over(iters, || {
        black_box(column_to_arrow(black_box(&small)).unwrap());
    }) / iters;
    let big_allocs = allocs_over(iters, || {
        black_box(column_to_arrow(black_box(&big)).unwrap());
    }) / iters;

    // The decisive proof of the one-bulk-copy path: identical allocation count at 100x the rows.
    assert_eq!(
        small_allocs, big_allocs,
        "column_to_arrow allocations must not scale with the row count \
         (1k => {small_allocs}, 100k => {big_allocs}) — a per-element copy would diverge"
    );
    // And that constant is tiny (the buffer copy + fixed Arrow overhead), not per-element.
    assert!(
        big_allocs <= 4,
        "a leaf column_to_arrow must copy once per buffer plus fixed overhead (got {big_allocs})"
    );
}
