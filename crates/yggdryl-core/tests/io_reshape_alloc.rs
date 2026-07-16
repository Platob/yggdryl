//! Deterministic allocation budget for the reshape seam. The claim under test: [`Serie::filter`] and
//! [`Serie::fill_null`] are **single-pass with bounded allocation** — each call allocates only the one
//! result buffer (a small constant), **not** an amount proportional to the number of rows. Proven by
//! running each op over a *small* and a *large* column and asserting the allocation count is
//! **identical** (size-independent), and a small constant per call.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::Serie;

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

fn allocs_over(iters: usize, mut op: impl FnMut()) -> usize {
    op();
    let before = ALLOCS.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    ALLOCS.load(Relaxed) - before
}

#[test]
fn filter_and_fill_null_are_single_pass_bounded_allocation() {
    let iters = 1000;

    // ---- filter: the allocation COUNT is independent of the column size (bounded, not per-row) ----
    // A no-null column keeps its validity mask absent, so `filter` allocates only the pre-sized value
    // buffer — a small constant, whatever the row count. The masks are built ONCE, outside the loop.
    let small = Serie::from_values(&(0..8i64).collect::<Vec<_>>());
    let large = Serie::from_values(&(0..8192i64).collect::<Vec<_>>());
    let small_mask = vec![true; small.len()];
    let large_mask = vec![true; large.len()];

    let filter_small = allocs_over(iters, || {
        let _ = small.filter(&small_mask).unwrap();
    });
    let filter_large = allocs_over(iters, || {
        let _ = large.filter(&large_mask).unwrap();
    });
    assert_eq!(
        filter_small, filter_large,
        "Serie::filter must allocate a bounded constant, not per-row ({filter_small} vs {filter_large})"
    );
    assert!(
        filter_small <= 4 * iters,
        "Serie::filter allocates a small constant per call (got {filter_small} over {iters})"
    );

    // ---- fill_null: same, over a column WITH nulls (a null every other row) ----
    // The result is fully-present (mask dropped); the op copies the values once and overwrites the
    // null slots in place — the allocation count is again size-independent.
    let opts_small: Vec<Option<i64>> = (0..8).map(|i| (i % 2 == 0).then_some(i)).collect();
    let opts_large: Vec<Option<i64>> = (0..8192).map(|i| (i % 2 == 0).then_some(i)).collect();
    let fill_small = Serie::from_options(&opts_small);
    let fill_large = Serie::from_options(&opts_large);

    let fill_alloc_small = allocs_over(iters, || {
        let _ = fill_small.fill_null(0);
    });
    let fill_alloc_large = allocs_over(iters, || {
        let _ = fill_large.fill_null(0);
    });
    assert_eq!(
        fill_alloc_small, fill_alloc_large,
        "Serie::fill_null must allocate a bounded constant, not per-row \
         ({fill_alloc_small} vs {fill_alloc_large})"
    );
    assert!(
        fill_alloc_small <= 4 * iters,
        "Serie::fill_null allocates a small constant per call (got {fill_alloc_small} over {iters})"
    );

    // ---- the no-null fill_null path is a clone (Arc bump), still bounded and size-independent ----
    let dense_small = Serie::from_values(&(0..8i64).collect::<Vec<_>>());
    let dense_large = Serie::from_values(&(0..8192i64).collect::<Vec<_>>());
    let clone_small = allocs_over(iters, || {
        let _ = dense_small.fill_null(0);
    });
    let clone_large = allocs_over(iters, || {
        let _ = dense_large.fill_null(0);
    });
    assert_eq!(
        clone_small, clone_large,
        "Serie::fill_null no-null clone path is size-independent ({clone_small} vs {clone_large})"
    );
    assert!(
        clone_small <= 4 * iters,
        "Serie::fill_null no-null clone path is bounded (got {clone_small})"
    );
}
