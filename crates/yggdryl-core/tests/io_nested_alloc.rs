//! Deterministic allocation budgets for the `io::nested` layer. Allocation counts are optimizer-
//! and machine-independent, so they assert the zero-copy claims directly: navigating a struct
//! column — borrowing a child [`Column`], reading its `len` / `type_id` / null count, looking a
//! field up by name — touches **no** heap (the data lives in the columns; navigation is borrows and
//! integer reads).
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::boxed;
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::nested::StructSerie;
use yggdryl_core::io::var::Utf8Serie;

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
    let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
    let names = boxed(Utf8Serie::from_strs(&[Some("a"), None, Some("c")]));
    let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

    // Borrowing a child column by index is a pointer read — no heap.
    let by_index = allocs_over(iters, || {
        let _ = table.column(0);
        let _ = table.column(1);
    });
    assert_eq!(
        by_index, 0,
        "StructSerie::column must be zero-copy (got {by_index})"
    );

    // Looking a child column up by name scans the field names (a `&str` compare) — no heap.
    let by_name = allocs_over(iters, || {
        let _ = table.column_named("name");
    });
    assert_eq!(
        by_name, 0,
        "StructSerie::column_named must be zero-copy (got {by_name})"
    );

    // Reading a column's shape (len / type_id / null count) is integer work — no heap.
    let column = table.column(1).unwrap();
    let shape = allocs_over(iters, || {
        let _ = column.len();
        let _ = column.type_id();
        let _ = column.null_count();
        let _ = column.has_nulls();
    });
    assert_eq!(
        shape, 0,
        "Column shape reads must be zero-copy (got {shape})"
    );

    // Reading a field descriptor by index is a borrow — no heap.
    let field = allocs_over(iters, || {
        let _ = table.field(0);
        let _ = table.len();
        let _ = table.null_count();
    });
    assert_eq!(
        field, 0,
        "StructSerie::field / len / null_count must be zero-copy (got {field})"
    );
}
