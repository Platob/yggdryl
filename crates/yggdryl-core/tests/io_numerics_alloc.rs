//! Deterministic allocation budgets for the broadened numeric surface — the wide `[u8; N]`
//! newtypes and the runtime-`N` fixed-size byte family. Allocation counts are optimizer- and
//! machine-independent, so they assert the zero-copy claims directly: the typed view, an element
//! read, and a fixed-size slice fetch all touch **no** heap.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::{Buffer, FixedBinarySerie, I256Serie, U128Scalar, I256};

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

    // A wide-newtype buffer: the typed reinterpret and element read are zero-copy.
    let values: Vec<I256> = (0..256)
        .map(|i| I256::from_le_bytes([i as u8; 32]))
        .collect();
    let buffer = Buffer::<I256>::from_slice(&values);
    let as_slice = allocs_over(iters, || {
        let _ = buffer.as_slice().len();
    });
    assert_eq!(
        as_slice, 0,
        "Buffer::<I256>::as_slice must be zero-copy (got {as_slice})"
    );
    let get = allocs_over(iters, || {
        let _ = buffer.get(128);
    });
    assert_eq!(get, 0, "Buffer::<I256>::get must be zero-copy (got {get})");

    // Reading a wide-newtype column element is a decode from borrowed bytes.
    let serie = I256Serie::from_options(&[Some(values[0]), None, Some(values[1])]);
    let serie_get = allocs_over(iters, || {
        let _ = serie.get(0);
        let _ = serie.get(1);
    });
    assert_eq!(
        serie_get, 0,
        "I256Serie::get must not allocate (got {serie_get})"
    );

    // A fixed-size binary column: `get_bytes` borrows a slot, no heap.
    let mut fixed = FixedBinarySerie::new(4);
    for chunk in [[1, 2, 3, 4], [5, 6, 7, 8]] {
        fixed.push(Some(&chunk)).unwrap();
    }
    let get_bytes = allocs_over(iters, || {
        let _ = fixed.get_bytes(0);
        let _ = fixed.get_bytes(1);
    });
    assert_eq!(
        get_bytes, 0,
        "FixedBinarySerie::get_bytes must be zero-copy (got {get_bytes})"
    );

    // A 128-bit scalar (de)serializes through a stack scratch — reading its value is heap-free.
    let scalar = U128Scalar::of(12345);
    let value = allocs_over(iters, || {
        let _ = scalar.value();
    });
    assert_eq!(
        value, 0,
        "U128Scalar::value must not allocate (got {value})"
    );
}
