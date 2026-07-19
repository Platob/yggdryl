//! Deterministic allocation budgets for the [`nested`](yggdryl_core::typed::nested) typed layer — the
//! fast, build-independent half of "validate both time and memory". Allocation *counts* do not depend
//! on the optimizer or the machine, so they can be asserted exactly and run in milliseconds, guarding
//! the **zero-allocation combine** ([`StructSerie::from_columns`] reuses the caller's `Vec<Column>`),
//! the **zero-allocation borrow lookups** (`column_by_name` / `column_path`), the **in-place deep
//! mutation** (`column_by_name_mut` + `set` allocates nothing per row), and the bounded row
//! materialize against regressions. (Throughput lives in the `typed_nested` bench.)
//!
//! This file is its own test binary with its own counting global allocator, and holds a **single**
//! `#[test]` so nothing else allocates on another thread while a region is measured. Counts are taken
//! over a tiny table (length 3) because every measured op's allocation count is **independent of the
//! row count** — the structural cost, not the data size, is what is guarded.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::typed::fixedbyte::{Int32, Int64};
use yggdryl_core::typed::varbyte::Utf8;
use yggdryl_core::typed::{Column, FixedSerie, StructSerie, VarSerie};

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

/// Total allocations `op` makes over `iters` runs, after one warm-up run so any one-time
/// initialization stays outside the measured window.
fn allocs_over(iters: usize, mut op: impl FnMut()) -> usize {
    op();
    let before = ALLOCS.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    ALLOCS.load(Relaxed) - before
}

/// Builds the reference erased columns: an `Int64` `id`, a `Utf8` `name`, and a nested `Struct`
/// `address` (a `Utf8` `city` + an `Int32` `zip`) — three columns of length 3. Each call owns the
/// leaf buffers fresh (used to measure the from-scratch build).
fn build_columns() -> Vec<Column> {
    let id = FixedSerie::<Int64>::from_values(&[10, 20, 30]).with_name("id");
    let name =
        VarSerie::<Utf8>::from_values(&["ada".into(), "bo".into(), "cy".into()]).with_name("name");
    let city = VarSerie::<Utf8>::from_values(&["paris".into(), "rome".into(), "oslo".into()])
        .with_name("city");
    let zip = FixedSerie::<Int32>::from_values(&[75001, 100, 3]).with_name("zip");
    let address = StructSerie::from_columns(vec![Column::from(city), Column::from(zip)])
        .unwrap()
        .with_name("address");
    vec![Column::from(id), Column::from(name), Column::from(address)]
}

#[test]
fn allocation_budgets() {
    let iters = 1000;

    // `from_columns` over a pre-built `Vec<Column>` is a **zero-allocation combine**: it takes
    // ownership of the caller's Vec (already paid for when the columns were assembled), sets an empty
    // `Headers` (an empty map allocates nothing) and no validity buffer — so the struct is formed with
    // no new allocation at all. Measured single-shot (the call consumes the Vec).
    let columns = build_columns();
    let before = ALLOCS.load(Relaxed);
    let table = StructSerie::from_columns(columns).unwrap();
    let combine = ALLOCS.load(Relaxed) - before;
    assert_eq!(
        combine, 0,
        "from_columns must reuse the caller's Vec with no new allocation (got {combine})"
    );

    // `column_by_name` walks the children and compares borrowed `&str` names — a pure borrow, zero
    // allocation across the whole lookup loop.
    let lookups = allocs_over(iters, || {
        black_box(table.column_by_name(black_box("name")));
    });
    assert_eq!(
        lookups, 0,
        "column_by_name must be a zero-alloc borrow (got {lookups} over {iters})"
    );

    // `column_path` descends a dotted path into a nested struct child — still only borrowed name
    // compares (`split_once` borrows), zero allocation.
    let paths = allocs_over(iters, || {
        black_box(table.column_path(black_box("address.city")));
    });
    assert_eq!(
        paths, 0,
        "column_path must be a zero-alloc borrow into the nested child (got {paths} over {iters})"
    );

    // `row(i)` materializes an owned `StructScalar`: the outer row owns its `names` + `values` Vecs
    // (one each) plus one `Box<str>`/owned value per child, and the nested struct child recurses into
    // its own row — a **small constant**, independent of the row count. The whole cost is guarded by a
    // tight upper bound (observed: 13 for this 3-column, one-nested table).
    let materialize = allocs_over(iters, || {
        black_box(table.row(black_box(1)));
    });
    let per_row = materialize / iters;
    assert!(
        per_row <= 13,
        "row(i) must materialize in a small constant number of allocations (got {per_row})"
    );

    // A **deep, in-place mutation**: recover the concrete `FixedSerie<Int64>` behind the erased
    // `&mut Column` and rewrite a row with a positioned write — no per-row allocation.
    let mut table_mut = StructSerie::from_columns(build_columns()).unwrap();
    let sets = allocs_over(iters, || {
        if let Some(Column::Int64(serie)) = table_mut.column_by_name_mut("id") {
            serie.set(0, black_box(999)).unwrap();
        }
    });
    assert_eq!(
        sets, 0,
        "column_by_name_mut + set must rewrite in place with no allocation (got {sets} over {iters})"
    );

    // The from-scratch build (leaf buffers + combine) owns exactly one growable backing per leaf plus
    // the columns' names and the two combining Vecs — a small, bounded constant at any row count. The
    // count is dominated by the leaf carriers each owning their data (and a var column its offsets),
    // never by the number of rows. (Observed: 22 allocations for this length-3, one-nested table —
    // the leaf data/offsets buffers, their names, the nested combine, and the two combining Vecs.)
    let full_build = allocs_over(1, || {
        let table = StructSerie::from_columns(build_columns()).unwrap();
        black_box(&table);
    });
    assert!(
        full_build <= 22,
        "the from-scratch nested build must own a bounded set of buffers (got {full_build})"
    );

    black_box((&table, &table_mut));
}
