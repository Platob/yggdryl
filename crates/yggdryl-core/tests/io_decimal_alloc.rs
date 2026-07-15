//! Deterministic allocation budgets for the decimal family — the zero-copy / no-alloc claims,
//! asserted directly through a counting global allocator (optimizer- and machine-independent):
//! value arithmetic and identity are stack-only, and a column element read decodes from borrowed
//! bytes without touching the heap.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::{D128Serie, D128, D256};

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

    // Value arithmetic is stack-only, even for the 256-bit width.
    let a = D256::new(123_456_789, 4).unwrap();
    let b = D256::new(987_654, 2).unwrap();
    let arithmetic = allocs_over(iters, || {
        let _ = a.checked_add(&b).unwrap();
        let _ = a.checked_mul(&b).unwrap();
    });
    assert_eq!(
        arithmetic, 0,
        "decimal arithmetic must not allocate (got {arithmetic})"
    );

    // Identity (equality / ordering) normalizes on the stack — no allocation.
    let x = D128::new(25, 1).unwrap();
    let y = D128::new(250, 2).unwrap();
    let identity = allocs_over(iters, || {
        let _ = x == y;
        let _ = x.cmp(&y);
    });
    assert_eq!(
        identity, 0,
        "decimal equality/ordering must not allocate (got {identity})"
    );

    // Writing the canonical bytes into a caller buffer is allocation-free.
    let mut scratch = [0u8; 1 + 16];
    let write = allocs_over(iters, || {
        x.write_serialized(&mut scratch);
    });
    assert_eq!(write, 0, "write_serialized must not allocate (got {write})");

    // Reading a column element decodes from borrowed bytes — no heap.
    let col = D128Serie::from_options(
        20,
        2,
        &[
            Some(D128::new(12345, 2).unwrap()),
            None,
            Some(D128::new(600, 2).unwrap()),
        ],
    )
    .unwrap();
    let get = allocs_over(iters, || {
        let _ = col.get(0);
        let _ = col.get_coeff(2);
        let _ = col.get(1); // null
    });
    assert_eq!(get, 0, "DecimalSerie::get must not allocate (got {get})");
}
