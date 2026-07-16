//! Deterministic allocation budget for the vectorized-arithmetic fast path. The claim under test:
//! a same-type `add_unchecked` and a scalar broadcast are **single-pass** with **bounded**
//! (size-independent) allocation — the result's value buffer + its `Arc` are the only allocations,
//! sized once up front, so a 64× larger column costs the **same** number of allocations per op (not
//! one-per-element). A per-element regression would make the large column allocate ~N× more.
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
    op(); // warm up (not counted)
    let before = ALLOCS.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    ALLOCS.load(Relaxed) - before
}

#[test]
fn unchecked_ops_are_single_pass_bounded_allocation() {
    let iters = 1000;

    // No-null columns at two very different sizes (64 vs 4096 elements).
    let small_a = Serie::from_values(&(0..64i64).collect::<Vec<_>>());
    let small_b = Serie::from_values(&(0..64i64).collect::<Vec<_>>());
    let large_a = Serie::from_values(&(0..4096i64).collect::<Vec<_>>());
    let large_b = Serie::from_values(&(0..4096i64).collect::<Vec<_>>());

    // ---- serie × serie -----------------------------------------------------------------
    let small = allocs_over(iters, || {
        let _ = small_a.add_unchecked(&small_b);
    });
    let large = allocs_over(iters, || {
        let _ = large_a.add_unchecked(&large_b);
    });
    // Size-independent: 64× the elements, the same allocation count per op.
    assert_eq!(
        small / iters,
        large / iters,
        "add_unchecked allocation must be size-independent (got {} vs {} per op)",
        small / iters,
        large / iters
    );
    // Bounded to a small constant per op (a per-element alloc would be ~N here).
    assert!(
        large / iters <= 4,
        "add_unchecked must be single-pass, bounded allocation (got {} per op)",
        large / iters
    );

    // ---- serie × scalar (broadcast) ----------------------------------------------------
    let small_s = allocs_over(iters, || {
        let _ = small_a.add_scalar_unchecked(1);
    });
    let large_s = allocs_over(iters, || {
        let _ = large_a.add_scalar_unchecked(1);
    });
    assert_eq!(
        small_s / iters,
        large_s / iters,
        "add_scalar_unchecked allocation must be size-independent (got {} vs {} per op)",
        small_s / iters,
        large_s / iters
    );
    assert!(
        large_s / iters <= 4,
        "add_scalar_unchecked must be single-pass, bounded allocation (got {} per op)",
        large_s / iters
    );
}
