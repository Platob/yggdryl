//! Deterministic allocation budget for the Phase 3 **grow** mutators — the anti-O(n²) proof.
//!
//! A bulk grow (`extend_values` / `concat`) builds the appended bytes into **one** pre-sized buffer
//! and commits them with a **single** copy-on-write append of the immutable values buffer — never one
//! re-seal per element. Allocation counts are optimizer- and machine-independent, so this asserts the
//! claim directly: growing a column by **2** vs by **1024** elements allocates the **same bounded
//! constant** number of times (the count is independent of how many elements are appended). A naive
//! per-element `push` loop would instead allocate `O(n)` times (one buffer re-seal per element).
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::temporal::{TimeUnit, Ts64, Tz};
use yggdryl_core::io::fixed::{D128Serie, I32Serie, Ts64Serie, D128};

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
fn bulk_grow_allocates_a_bounded_constant_not_per_element() {
    let iters = 1000;
    let small: Vec<i32> = (0..2).collect();
    let big: Vec<i32> = (0..1024).collect();
    // A fixed 2-element base is rebuilt fresh each op (so it uniquely owns its buffer and the append
    // is a single reused-then-resealed COW, not a copy of a shared Arc).
    let base: Vec<i32> = vec![7, 8];

    // ---- Serie<T>::extend_values: appending 2 vs 1024 costs the same allocation count -----
    let ext_small = allocs_over(iters, || {
        let mut col = I32Serie::from_values(&base);
        col.extend_values(&small);
    });
    let ext_big = allocs_over(iters, || {
        let mut col = I32Serie::from_values(&base);
        col.extend_values(&big);
    });
    assert_eq!(
        ext_small, ext_big,
        "Serie::extend_values must allocate a bounded constant, not O(n) \
         (2 elems: {ext_small}, 1024 elems: {ext_big})"
    );

    // ---- Serie<T>::concat: memcpy of a small vs large source costs the same count ---------
    let src_small = I32Serie::from_values(&small);
    let src_big = I32Serie::from_values(&big);
    let cat_small = allocs_over(iters, || {
        let mut col = I32Serie::from_values(&base);
        col.concat(&src_small);
    });
    let cat_big = allocs_over(iters, || {
        let mut col = I32Serie::from_values(&base);
        col.concat(&src_big);
    });
    assert_eq!(
        cat_small, cat_big,
        "Serie::concat must be a single COW, allocations independent of source length \
         (2 elems: {cat_small}, 1024 elems: {cat_big})"
    );

    // ---- DecimalSerie::extend_values: the ArrowBuffer re-seal happens once ----------------
    let dsmall: Vec<D128> = (0..2).map(|v| D128::new(v as i128, 2).unwrap()).collect();
    let dbig: Vec<D128> = (0..1024)
        .map(|v| D128::new(v as i128, 2).unwrap())
        .collect();
    let dbase = D128Serie::from_values(20, 2, &dsmall).unwrap();
    let dec_small = allocs_over(iters, || {
        let mut col = dbase.clone();
        col.extend_values(&dsmall).unwrap();
    });
    let dec_big = allocs_over(iters, || {
        let mut col = dbase.clone();
        col.extend_values(&dbig).unwrap();
    });
    assert_eq!(
        dec_small, dec_big,
        "DecimalSerie::extend_values must re-seal the coefficient buffer once, not per element \
         (2 elems: {dec_small}, 1024 elems: {dec_big})"
    );

    // ---- TemporalSerie::extend_values: likewise one re-seal of the counts buffer ----------
    let tsmall: Vec<Ts64> = (0..2)
        .map(|v| Ts64::from_epoch(v, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let tbig: Vec<Ts64> = (0..1024)
        .map(|v| Ts64::from_epoch(v, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let tbase = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &tsmall).unwrap();
    let ts_small = allocs_over(iters, || {
        let mut col = tbase.clone();
        col.extend_values(&tsmall).unwrap();
    });
    let ts_big = allocs_over(iters, || {
        let mut col = tbase.clone();
        col.extend_values(&tbig).unwrap();
    });
    assert_eq!(
        ts_small, ts_big,
        "TemporalSerie::extend_values must re-seal the counts buffer once, not per element \
         (2 elems: {ts_small}, 1024 elems: {ts_big})"
    );
}
