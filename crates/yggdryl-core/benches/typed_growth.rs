//! Time **and** memory benchmark for the **serie growth + reshape** surface — capacity-aware
//! building, repeated-value fills, column concatenation, null-filling, compaction, reversal, and
//! sorting over a [`FixedSerie`]. The **allocs/op** column is the point of most rows: it shows the
//! **capacity win** (pre-sizing collapses a growing build's `log2(N)` reallocations to one) and the
//! **alloc-constant fills** (`repeat` / `push_repeat` never materialize the `count`-element array,
//! so they allocate only the result's own data buffer — the fill itself is allocation-free).
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! other benches. Run with `cargo bench -p yggdryl-core --bench typed_growth`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::typed::fixedbyte::{Int32, Int64};
use yggdryl_core::typed::{FixedSerie, Scalar};

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
    println!("  {name:<44} {mops:8.1}    {allocs:8.2}    {bytes:10.0}");
}

fn main() {
    let iters = 1_000;
    let n = 1 << 16; // 65 536 elements

    let ints: Vec<i64> = (0..n as i64).collect();
    // A pseudo-shuffled column so sorting does real work, and a nullable column for fill_null.
    let shuffled: Vec<i32> = (0..n as i32)
        .map(|i| (i * 2_654_435_761u32 as i32) ^ 0x5bd1)
        .collect();
    let shuffled_col = FixedSerie::<Int32>::from_values(&shuffled);
    let nullable = FixedSerie::<Int32>::from_options(
        &(0..n as i32)
            .map(|i| if i % 4 == 0 { None } else { Some(i) })
            .collect::<Vec<_>>(),
    );
    let source = FixedSerie::<Int64>::from_values(&ints);
    let mut mask = yggdryl_core::io::memory::Heap::new();
    {
        use yggdryl_core::io::memory::IOBase;
        // keep every other element
        let bytes = vec![0b1010_1010u8; n / 8];
        mask.pwrite_byte_array(0, &bytes);
    }

    println!("serie growth + reshape — time & memory ({iters} iters over {n} elements)\n");
    println!(
        "  {:<44} {:>8}    {:>8}    {:>10}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(80));

    // -- Capacity win: the same build, grown vs pre-sized -----------------------------------
    println!("  -- build: capacity win (push loop grow vs pre-sized vs bulk append) --");
    row(
        "push loop, NO capacity (grows ~log2 N)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::new();
            for &value in &ints {
                s.push(black_box(value));
            }
            black_box(s.len());
        }),
    );
    row(
        "push loop, with_capacity (1 alloc)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            for &value in &ints {
                s.push(black_box(value));
            }
            black_box(s.len());
        }),
    );
    row(
        "with_capacity + append (bulk, 1 alloc)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            s.append(black_box(&ints));
            black_box(s.len());
        }),
    );

    // -- Alloc-constant fills: the count-element array is never materialized -----------------
    println!("\n  -- fill: repeat / push_repeat (alloc-constant, no materialized array) --");
    row(
        "repeat(value, N) builder",
        measure(n, iters, || {
            black_box(FixedSerie::<Int64>::repeat(black_box(7), n));
        }),
    );
    row(
        "push_repeat onto pre-sized column",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            s.push_repeat(black_box(7), n);
            black_box(s.len());
        }),
    );

    // -- Concatenation: one bulk copy of another column's data ------------------------------
    println!("\n  -- concat: append slice / extend serie (one bulk copy) --");
    row(
        "extend (serie into pre-sized serie)",
        measure(n, iters, || {
            let mut s = FixedSerie::<Int64>::with_capacity(n);
            s.extend(black_box(&source));
            black_box(s.len());
        }),
    );

    // -- Reshape over the raw buffer --------------------------------------------------------
    println!("\n  -- reshape: fill_null / mask_filter / reverse --");
    row(
        "fill_null (nullable -> non-nullable copy)",
        measure(n, iters, || {
            black_box(nullable.fill_null(black_box(-1)));
        }),
    );
    row(
        "mask_filter (keep every other, compact)",
        measure(n, iters, || {
            black_box(source.mask_filter(black_box(&mask)));
        }),
    );
    row(
        "reverse (dense reversed copy)",
        measure(n, iters, || {
            black_box(source.reverse());
        }),
    );

    // -- Sort: the permutation, then the gather --------------------------------------------
    println!("\n  -- sort: sort_indices (permutation) + take (gather) --");
    let sort_iters = 100;
    row(
        "sort_indices (stable permutation)",
        measure(n, sort_iters, || {
            black_box(shuffled_col.sort_indices(black_box(true)));
        }),
    );
    row(
        "sort = take(sort_indices(true))",
        measure(n, sort_iters, || {
            black_box(shuffled_col.sort());
        }),
    );
}
