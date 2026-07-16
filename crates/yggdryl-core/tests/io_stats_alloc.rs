//! Deterministic allocation budget for the analytics seams. The claim under test: streaming
//! iteration ([`Serie::iter`] / [`iter_valid`]) and the [`NumericSerie`] fold reductions (sum / min
//! / max / mean / count) are **allocation-free** — they read the column in place and never
//! materialize. Only the explicit `to_f64_*` collectors allocate (one `Vec`), which is their job.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::NumericSerie;

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
fn reductions_are_allocation_free() {
    let iters = 1000;
    let col = Serie::from_options(&[Some(1i64), None, Some(2), Some(3), None, Some(9)]);

    // Streaming iteration touches no heap.
    let iter_allocs = allocs_over(iters, || {
        let _ = col.iter().filter(|v| v.is_some()).count();
        let _ = col.iter_valid().count();
    });
    assert_eq!(
        iter_allocs, 0,
        "Serie::iter / iter_valid must be allocation-free (got {iter_allocs})"
    );

    // The fold reductions touch no heap.
    let reduce_allocs = allocs_over(iters, || {
        let _ = col.valid_count();
        let _ = col.sum_f64();
        let _ = col.mean_f64();
        let _ = col.min_f64();
        let _ = col.max_f64();
    });
    assert_eq!(
        reduce_allocs, 0,
        "NumericSerie reductions must be allocation-free (got {reduce_allocs})"
    );
}
