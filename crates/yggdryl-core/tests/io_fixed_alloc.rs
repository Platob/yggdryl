//! Deterministic allocation budgets for the `io::fixed` typed layer. Allocation counts are
//! optimizer- and machine-independent, so they assert the zero-copy claims directly: reading
//! an element, taking a typed view, and decoding a `Scalar` / `Serie` element from a byte
//! stream all touch **no** heap.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::{Buffer, I32Scalar, I32Serie};
use yggdryl_core::io::{Bytes, IOCursor};

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
    let buffer = Buffer::<i32>::from_vec((0..1024).collect());

    // Decoding one element from the borrowed bytes is a read, not an allocation.
    let get = allocs_over(iters, || {
        let _ = buffer.get(512);
    });
    assert_eq!(get, 0, "Buffer::get must be zero-copy (got {get})");

    // The typed view is a reinterpret of the shared allocation — no heap.
    let as_slice = allocs_over(iters, || {
        let _ = buffer.as_slice().len();
    });
    assert_eq!(
        as_slice, 0,
        "Buffer::as_slice must be zero-copy (got {as_slice})"
    );

    // A scalar decodes into a stack frame — reading it from a stream touches no heap.
    let mut scalar_stream = Bytes::new();
    for _ in 0..8 {
        I32Scalar::of(7).write_to(&mut scalar_stream).unwrap();
    }
    let read_scalar = allocs_over(iters, || {
        scalar_stream.rewind();
        let _ = I32Scalar::read_from(&mut scalar_stream).unwrap();
    });
    assert_eq!(
        read_scalar, 0,
        "Scalar::read_from must not allocate (got {read_scalar})"
    );

    // Reading a column element (validity bit + value) is a decode, not an allocation.
    let serie = I32Serie::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    let serie_get = allocs_over(iters, || {
        let _ = serie.get(3);
        let _ = serie.get(4);
    });
    assert_eq!(
        serie_get, 0,
        "Serie::get must be zero-copy (got {serie_get})"
    );
}
