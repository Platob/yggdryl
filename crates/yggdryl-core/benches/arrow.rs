//! Time **and** memory benchmark for the Apache Arrow interop bridge
//! ([`crate::arrow`](yggdryl_core::arrow), feature `arrow`) — the leaf
//! [`column_to_arrow`](yggdryl_core::arrow::column_to_arrow) /
//! [`column_from_arrow`](yggdryl_core::arrow::column_from_arrow) conversions and the top-level
//! [`struct_serie_to_record_batch`](yggdryl_core::arrow::struct_serie_to_record_batch) /
//! [`struct_serie_from_record_batch`](yggdryl_core::arrow::struct_serie_from_record_batch)
//! round-trip. The point: the handoff is **one buffer copy** per column buffer — the entry point
//! borrows the `&Column`, so its owning `Heap` cannot be moved into a zero-copy `Buffer::from_vec`,
//! and the from-Arrow direction re-encodes the logical values once (respecting a sliced input). The
//! **allocs/op** column proves the copy count is a small constant per buffer, independent of the row
//! count.
//!
//! Dependency-free harness (`harness = false`, a plain `main`) with the same counting allocator as
//! the other benches; `required-features = ["arrow"]` gates it. Run with
//! `cargo bench -p yggdryl-core --features arrow --bench arrow`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::arrow::{
    column_from_arrow, column_to_arrow, struct_serie_from_record_batch,
    struct_serie_to_record_batch,
};
use yggdryl_core::typed::fixedbyte::Int64;
use yggdryl_core::typed::varbyte::Utf8;
use yggdryl_core::typed::{Column, FixedSerie, StructSerie, VarSerie};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
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

fn measure(items: usize, iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = items as f64 * f64::from(iters);
    (
        total / secs / 1_000_000.0,
        (a1 - a0) as f64 / f64::from(iters),
        (b1 - b0) as f64 / f64::from(iters),
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<44} {mops:8.1}    {allocs:8.2}    {bytes:11.0}");
}

fn main() {
    let iters = 2_000;
    let n = 100_000; // rows

    let ids: Vec<i64> = (0..n as i64).collect();
    let names: Vec<String> = (0..n).map(|i| format!("u{i}")).collect();

    // Leaf columns: a large Int64 (fixed-width, reinterpreted one-copy) and a large Utf8 (offsets +
    // data, one copy each).
    let int_col = Column::from(FixedSerie::<Int64>::from_values(&ids));
    let int_field = int_col.field();
    let utf8_col = Column::from(VarSerie::<Utf8>::from_values(&names));
    let utf8_field = utf8_col.field();

    let int_array = column_to_arrow(&int_col).unwrap();
    let utf8_array = column_to_arrow(&utf8_col).unwrap();

    println!("arrow interop — time & memory ({iters} iters over {n} rows)\n");
    println!(
        "  {:<44} {:>8}    {:>8}    {:>11}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(84));

    // -- Leaf column <-> Arrow array (the one-copy buffer handoff) --------------------------------
    println!("  -- leaf column <-> Arrow array --");
    row(
        "column_to_arrow   i64  (reinterpret, 1 copy)",
        measure(n, iters, || {
            black_box(column_to_arrow(black_box(&int_col)).unwrap());
        }),
    );
    row(
        "column_from_arrow i64  (re-encode)",
        measure(n, iters, || {
            black_box(column_from_arrow(black_box(&int_array), &int_field).unwrap());
        }),
    );
    row(
        "column_to_arrow   utf8 (offsets+data, 1 copy)",
        measure(n, iters, || {
            black_box(column_to_arrow(black_box(&utf8_col)).unwrap());
        }),
    );
    row(
        "column_from_arrow utf8 (rebuild by value)",
        measure(n, iters / 4, || {
            black_box(column_from_arrow(black_box(&utf8_array), &utf8_field).unwrap());
        }),
    );

    // -- StructSerie <-> RecordBatch (the top-level table bridge) ---------------------------------
    println!("\n  -- struct \"table\" <-> RecordBatch --");
    let table = StructSerie::from_columns(vec![
        Column::from(FixedSerie::<Int64>::from_values(&ids).with_name("id")),
        Column::from(VarSerie::<Utf8>::from_values(&names).with_name("name")),
        Column::from(FixedSerie::<Int64>::from_values(&ids).with_name("amount")),
    ])
    .unwrap();
    let batch = struct_serie_to_record_batch(&table).unwrap();
    row(
        "struct_serie_to_record_batch   (3 cols)",
        measure(n, iters, || {
            black_box(struct_serie_to_record_batch(black_box(&table)).unwrap());
        }),
    );
    row(
        "struct_serie_from_record_batch (3 cols)",
        measure(n, iters / 4, || {
            black_box(struct_serie_from_record_batch(black_box(&batch)).unwrap());
        }),
    );

    black_box((&int_array, &utf8_array, &batch));
}
