//! **The allocation proof of Phase 10** (the load-bearing part). The claim: an in-place `*_assign`
//! twin mutates self's buffer through copy-on-write, so on a **uniquely-owned** column it **never
//! copies the payload** — the values are mutated where they live — while on a **shared** column (a
//! shallow clone held alive) it pays exactly the one copy-on-write of the payload, like the
//! return-new `add`, which always materializes a fresh result.
//!
//! The honest measure of "no heavy copy" is **bytes allocated, not allocation count**: reusing an
//! owned Arrow allocation still costs a couple of tiny `Arc` header allocations (the owned path is
//! not literally zero-alloc — Arrow re-wraps the reused allocation), but it copies **none of the
//! payload**. So the test asserts the owned in-place path allocates a **small, size-INDEPENDENT**
//! number of bytes (identical for a 64- and a 4096-element column), whereas the shared / return-new
//! paths allocate bytes that **scale with the payload** (the copy).
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::Serie;

struct Counting;
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            BYTES.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Bytes allocated over `iters` calls of `op`, after one un-counted warm-up call.
fn bytes_over(iters: usize, mut op: impl FnMut()) -> usize {
    op(); // warm up (not counted)
    let before = BYTES.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    BYTES.load(Relaxed) - before
}

#[test]
fn in_place_ops_do_not_copy_the_payload() {
    let iters = 1000;
    let big: Vec<i64> = (0..4096).collect(); // 32 KiB payload
    let small: Vec<i64> = (0..64).collect(); //  512 B payload
    let payload_gap = (big.len() - small.len()) * std::mem::size_of::<i64>(); // ~31.5 KiB / op
    let addend = Serie::from_values(&big);
    let addend_small = Serie::from_values(&small);

    // ---- add_assign on a UNIQUELY-OWNED column: the payload is NOT copied ----------------------
    // Bytes allocated are size-INDEPENDENT (the same for a 64× larger payload) — the values are
    // mutated in the existing allocation; only a couple of tiny Arc headers are (re)allocated.
    let mut owned = Serie::from_values(&big);
    let owned_big = bytes_over(iters, || owned.add_assign(&addend)) / iters;
    let mut owned_small = Serie::from_values(&small);
    let owned_small = bytes_over(iters, || owned_small.add_assign(&addend_small)) / iters;
    assert_eq!(
        owned_big, owned_small,
        "owned add_assign must not copy the payload — per-op bytes must be size-independent \
         (big {owned_big} vs small {owned_small})"
    );
    assert!(
        owned_big < payload_gap,
        "owned add_assign per-op bytes ({owned_big}) must be far below the payload size — no copy"
    );

    // ---- add_assign on a SHARED column: the one copy-on-write of the payload -------------------
    // A shallow clone held alive forces each call to copy the buffer once; bytes now SCALE with the
    // payload (big allocates ~payload_gap more than small).
    let mut shared = Serie::from_values(&big);
    let shared_big = bytes_over(iters, || {
        let _keep = shared.clone(); // shallow: shares the values Arc
        shared.add_assign(&addend); // shared → one COW of the payload
    }) / iters;
    let mut shared_small = Serie::from_values(&small);
    let shared_small = bytes_over(iters, || {
        let _keep = shared_small.clone();
        shared_small.add_assign(&addend_small);
    }) / iters;
    assert!(
        shared_big >= shared_small + payload_gap,
        "shared add_assign copies the payload — big ({shared_big}) must exceed small \
         ({shared_small}) by ~the payload gap ({payload_gap})"
    );
    // And the owned path allocates dramatically fewer bytes than the shared copy on the big column.
    assert!(
        owned_big * 8 < shared_big,
        "owned in-place ({owned_big} B/op) must allocate far less than the shared COW \
         ({shared_big} B/op) — it copies no payload"
    );

    // ---- contrast: return-new add_unchecked also materializes (copies into) a fresh result -----
    let owned_for_ret = Serie::from_values(&big);
    let ret_big = bytes_over(iters, || {
        let _result = owned_for_ret.add_unchecked(&addend);
    }) / iters;
    assert!(
        ret_big >= payload_gap,
        "return-new add allocates a fresh result payload ({ret_big} B/op) — far above owned in-place"
    );

    // ---- add_scalar_assign on an owned column: size-independent (no payload copy) --------------
    let mut owned_scalar = Serie::from_values(&big);
    let scalar_big = bytes_over(iters, || owned_scalar.add_scalar_assign(1)) / iters;
    let mut owned_scalar_small = Serie::from_values(&small);
    let scalar_small = bytes_over(iters, || owned_scalar_small.add_scalar_assign(1)) / iters;
    assert_eq!(
        scalar_big, scalar_small,
        "owned add_scalar_assign is size-independent (big {scalar_big} vs small {scalar_small})"
    );
    assert!(
        scalar_big < payload_gap,
        "owned add_scalar_assign copies no payload"
    );
}
