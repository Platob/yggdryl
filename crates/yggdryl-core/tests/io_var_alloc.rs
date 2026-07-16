//! Deterministic allocation budgets for the `io::var` variable-length layer. Allocation counts
//! are optimizer- and machine-independent, so they assert the zero-copy claims directly: the
//! `get_str` / `get_bytes` / `value_bytes` / `as_str` accessors all hand back a **borrow** into
//! the column's data buffer and touch **no** heap.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::var::{BinarySerie, Utf8Scalar, Utf8Serie, VarScalar, VarSerie};

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
fn allocation_budgets() {
    let iters = 1000;
    let col = Utf8Serie::from_strs(&[Some("alpha"), None, Some(""), Some("delta")]);

    // Fetching an element's `&str` is a borrow into the data buffer — no heap.
    let get_str = allocs_over(iters, || {
        let _ = col.get_str(0);
        let _ = col.get_str(3);
    });
    assert_eq!(
        get_str, 0,
        "Utf8Serie::get_str must be zero-copy (got {get_str})"
    );

    // The `VarSerie::value_bytes` trait accessor is the same borrow.
    let value_bytes = allocs_over(iters, || {
        let _ = VarSerie::value_bytes(&col, 0);
    });
    assert_eq!(
        value_bytes, 0,
        "VarSerie::value_bytes must be zero-copy (got {value_bytes})"
    );

    // Arbitrary binary bytes, same story.
    let bin = BinarySerie::from_options(&[Some(&[0xff, 0x00][..]), Some(&[0x01][..])]).unwrap();
    let get_bytes = allocs_over(iters, || {
        let _ = bin.get_bytes(0);
        let _ = bin.get_bytes(1);
    });
    assert_eq!(
        get_bytes, 0,
        "BinarySerie::get_bytes must be zero-copy (got {get_bytes})"
    );

    // A present scalar's value is stored inline; reading it back as `&str` / `&[u8]` borrows.
    let scalar = Utf8Scalar::of("held value");
    let as_str = allocs_over(iters, || {
        let _ = scalar.as_str();
        let _ = VarScalar::value_bytes(&scalar);
    });
    assert_eq!(
        as_str, 0,
        "Utf8Scalar::as_str / value_bytes must be zero-copy (got {as_str})"
    );
}
