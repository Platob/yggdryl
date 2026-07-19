//! Time **and** memory benchmark for the isolated any→any converter
//! ([`convert_column`](yggdryl_core::typed::convert_column)). It shows the shape of each arm's cost:
//!
//! - **i64 → i32** (resize) and **decimal128 → i128** (unscaled relabel) and **binary → utf8**
//!   (offsets+data reinterpret) allocate a **small constant** number of buffers — their allocation
//!   count does **not** grow with the row count `N` (the reinterpret is byte-for-byte, the relabel is
//!   zero-copy over the physical, the resize stages one output buffer).
//! - **i64 → utf8** (format) and **utf8 → i64** (flexible parse) allocate **per element** — that is
//!   inherent to materializing / reading `N` owned strings, not a converter overhead.
//! - **bool → i8** unpacks the bit column into one values buffer and one output buffer.
//!
//! The final section proves the constancy directly: it measures the allocation count at `N` and at
//! `4·N` for the reinterpret / relabel / resize arms and reports them side by side — equal counts
//! mean the path is **alloc-constant in N**.
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the other
//! benches. Run with `cargo bench -p yggdryl-core --bench typed_convert`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{Decimal128, Int64};
use yggdryl_core::typed::{convert_column, Column, FixedSerie};

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

/// Throughput (Melem/s), allocations/op, and bytes/op for `op`, after one warm-up run.
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
    println!("  {name:<40} {mops:9.1}    {allocs:9.2}    {bytes:12.0}");
}

/// The exact allocation count of one `op` run (warm-up excluded) — for the "alloc-constant in N" proof.
fn allocs_once(mut op: impl FnMut()) -> usize {
    op();
    let before = ALLOCS.load(Relaxed);
    op();
    ALLOCS.load(Relaxed) - before
}

fn main() {
    let iters = 2_000;
    let n = 100_000;

    // Pre-built source columns, so each measured op times the conversion, not the source build.
    let ints = Column::from(FixedSerie::<Int64>::from_values(
        &(0..n as i64).collect::<Vec<_>>(),
    ));
    let decimals = Column::from(FixedSerie::<Decimal128>::from_values(
        &(0..n as i128).map(|v| v * 100 + 5).collect::<Vec<_>>(),
    ));
    let bools = Column::from(FixedSerie::<Bit>::from_values(
        &(0..n).map(|i| i % 3 == 0).collect::<Vec<_>>(),
    ));
    let text = convert_column(&ints, DataTypeId::Utf8, None).unwrap();
    let binary = convert_column(&text, DataTypeId::Binary, None).unwrap();

    println!("typed convert — time & memory ({iters} iters over {n} rows)\n");
    println!(
        "  {:<40} {:>9}    {:>9}    {:>12}",
        "conversion", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(80));

    row(
        "i64 -> i32 (resize)",
        measure(n, iters, || {
            black_box(convert_column(black_box(&ints), DataTypeId::I32, None).unwrap());
        }),
    );
    row(
        "decimal128 -> i128 (unscaled relabel)",
        measure(n, iters, || {
            black_box(convert_column(black_box(&decimals), DataTypeId::I128, None).unwrap());
        }),
    );
    row(
        "binary -> utf8 (reinterpret)",
        measure(n, iters, || {
            black_box(convert_column(black_box(&binary), DataTypeId::Utf8, None).unwrap());
        }),
    );
    row(
        "bool -> i8 (bit unpack)",
        measure(n, iters / 4, || {
            black_box(convert_column(black_box(&bools), DataTypeId::I8, None).unwrap());
        }),
    );
    row(
        "i64 -> utf8 (format)",
        measure(n, iters / 8, || {
            black_box(convert_column(black_box(&ints), DataTypeId::Utf8, None).unwrap());
        }),
    );
    row(
        "utf8 -> i64 (flexible parse)",
        measure(n, iters / 8, || {
            black_box(convert_column(black_box(&text), DataTypeId::I64, None).unwrap());
        }),
    );

    // -- alloc-constant-in-N proof: the same op at N and 4·N must allocate the same count ----------
    println!("\n  allocations vs row count (constant ⇒ does not scale with N):");
    println!("    {:<40} {:>10} {:>10}", "conversion", "at N", "at 4·N");
    let big = 4 * n;
    let ints_big = Column::from(FixedSerie::<Int64>::from_values(
        &(0..big as i64).collect::<Vec<_>>(),
    ));
    let decimals_big = Column::from(FixedSerie::<Decimal128>::from_values(
        &(0..big as i128).map(|v| v * 100 + 5).collect::<Vec<_>>(),
    ));
    let text_big = convert_column(&ints_big, DataTypeId::Utf8, None).unwrap();
    let binary_big = convert_column(&text_big, DataTypeId::Binary, None).unwrap();

    let pairs: [(&str, &Column, &Column, DataTypeId); 3] = [
        ("i64 -> i32 (resize)", &ints, &ints_big, DataTypeId::I32),
        (
            "decimal128 -> i128 (relabel)",
            &decimals,
            &decimals_big,
            DataTypeId::I128,
        ),
        (
            "binary -> utf8 (reinterpret)",
            &binary,
            &binary_big,
            DataTypeId::Utf8,
        ),
    ];
    for (name, small, large, to) in pairs {
        let at_n = allocs_once(|| {
            black_box(convert_column(black_box(small), to, None).unwrap());
        });
        let at_4n = allocs_once(|| {
            black_box(convert_column(black_box(large), to, None).unwrap());
        });
        println!("    {name:<40} {at_n:>10} {at_4n:>10}");
    }

    black_box((&ints, &decimals, &bools, &text, &binary));
}
