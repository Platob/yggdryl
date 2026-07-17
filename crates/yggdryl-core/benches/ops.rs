//! Time **and** memory benchmark for the Phase 8b vectorized arithmetic: the typed fast path
//! (`Serie::add_unchecked` / the scalar broadcast) against the erased, checking + casting base ops
//! (`dyn AnySerie::add`), and — the headline — the **cast cost** of a cross-type add (`i32 + i64`,
//! whose right operand is range-checked into the left's `i32`) over the same-type add.
//!
//! Dependency-free harness (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench ops`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Field, Serie};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId};

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
    println!("  {name:<44} {mops:8.1}      {allocs:6.2}      {bytes:9.1}");
}

fn header(title: &str) {
    println!("\n{title}\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(84));
}

fn main() {
    let iters = 20_000;
    let n = 4096usize;

    let a: Vec<i32> = (0..n as i32).collect();
    let b: Vec<i32> = (1..=n as i32).collect();
    let a32 = Serie::from_values(&a);
    let b32 = Serie::from_values(&b);
    let b64 = Serie::from_values(&(1..=n as i64).collect::<Vec<_>>());

    // Erased forms for the base-op rows.
    let ea = boxed(Serie::from_values(&a));
    let eb32 = boxed(Serie::from_values(&b));
    let eb64 = boxed(Serie::from_values(&(1..=n as i64).collect::<Vec<_>>()));
    let scalar = AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        7i32.to_le_bytes().to_vec(),
    );

    header(&format!(
        "Vectorized arithmetic — time & memory ({iters} iters, {n} elements/op)"
    ));

    // The typed fast path — one tight pass, one result buffer.
    row(
        "Serie::add_unchecked (typed, same T)",
        measure(n, iters, || {
            let _ = a32.add_unchecked(&b32);
        }),
    );
    row(
        "Serie::div_unchecked (typed, zero-check)",
        measure(n, iters, || {
            let _ = a32.div_unchecked(&b32);
        }),
    );
    row(
        "Serie::add_scalar_unchecked (typed)",
        measure(n, iters, || {
            let _ = a32.add_scalar_unchecked(7);
        }),
    );

    // In-place COW twins: `add_assign` on a uniquely-owned column mutates the buffer where it lives
    // (allocation-free) and is at least as fast as the return-new `add_unchecked` (no result buffer
    // to allocate + fill). A shared column pays one copy-on-write, matching the return-new cost.
    {
        let mut owned = Serie::from_values(&a);
        row(
            "Serie::add_assign (typed, in-place OWNED — 0 alloc)",
            measure(n, iters, || {
                owned.add_assign(&b32);
            }),
        );
    }
    {
        let mut shared = Serie::from_values(&a);
        row(
            "Serie::add_assign (typed, in-place SHARED — 1 COW)",
            measure(n, iters, || {
                let _keep = shared.clone(); // hold a shallow clone alive → shared buffer
                shared.add_assign(&b32);
            }),
        );
    }
    {
        let mut owned = Serie::from_values(&a);
        row(
            "Serie::add_scalar_assign (typed, in-place OWNED)",
            measure(n, iters, || {
                owned.add_scalar_assign(7);
            }),
        );
    }

    // The erased base op — same type (no cast) vs cross-type (the range-checked cast into the left).
    row(
        "dyn AnySerie::add (erased, same T)",
        measure(n, iters, || {
            let _ = ea.add(eb32.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add (erased, i32 + i64 — CAST)",
        measure(n, iters, || {
            let _ = ea.add(eb64.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add_scalar (erased broadcast)",
        measure(n, iters, || {
            let _ = ea.add_scalar(&scalar).unwrap();
        }),
    );

    // The typed cast in isolation, to attribute the cross-type overhead above.
    row(
        "Serie::<i64>::cast::<i32> (the cast alone)",
        measure(n, iters, || {
            let _ = b64.cast::<i32>().unwrap();
        }),
    );

    println!(
        "\n  Cross-type add pays one range-checked cast of the right operand into the left's type;\n  \
         the same-type add and the typed `*_unchecked` skip it (a single fused pass)."
    );

    // ---- nested ops: the erased add recurses to the leaf `*_unchecked` kernels -------------------
    //
    // A struct adds field-wise, a list element-wise (over the flattened child), a map value-wise —
    // each recursing into the vectorized leaf kernel above, plus a per-row validity rebuild. Fewer
    // iters: a nested op rebuilds the container (offsets / field schema) around the leaf work.
    let nested_iters = 2_000;

    // struct<x: i32, y: i32> over n rows (two numeric fields -> 2n element-ops per add).
    let struct_a = boxed(
        StructSerie::from_named(vec![
            ("x", boxed(Serie::from_values(&a))),
            ("y", boxed(Serie::from_values(&b))),
        ])
        .unwrap(),
    );
    let struct_b = boxed(
        StructSerie::from_named(vec![
            ("x", boxed(Serie::from_values(&b))),
            ("y", boxed(Serie::from_values(&a))),
        ])
        .unwrap(),
    );

    // list<i32> and map<i64, i32>, both over the same n leaf elements (2 items / entries per row).
    let offsets: Vec<i32> = (0..=n as i32).step_by(2).collect();
    let keys: Vec<i64> = (0..n as i64).collect();
    let list_a = boxed(
        ListSerie::from_values(Serie::from_values(&a).named("item"), &offsets, None).unwrap(),
    );
    let list_b = boxed(
        ListSerie::from_values(Serie::from_values(&b).named("item"), &offsets, None).unwrap(),
    );
    let map_a = boxed(
        MapSerie::from_entries(
            Serie::from_values(&keys).named("key"),
            Serie::from_values(&a).named("value"),
            &offsets,
            None,
            false,
        )
        .unwrap(),
    );
    let map_b = boxed(
        MapSerie::from_entries(
            Serie::from_values(&keys).named("key"),
            Serie::from_values(&b).named("value"),
            &offsets,
            None,
            false,
        )
        .unwrap(),
    );

    header(&format!(
        "Nested arithmetic — the erased add recurses to the leaf kernels ({nested_iters} iters, {n} \
         leaf elements/op)"
    ));
    row(
        "dyn AnySerie::add (struct, 2 numeric fields)",
        measure(n * 2, nested_iters, || {
            let _ = struct_a.add(struct_b.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add (list, element-wise)",
        measure(n, nested_iters, || {
            let _ = list_a.add(list_b.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add (map, value-wise)",
        measure(n, nested_iters, || {
            let _ = map_a.add(map_b.as_ref()).unwrap();
        }),
    );

    println!(
        "\n  BEFORE/AFTER: the leaf `*_unchecked` kernels the nested ops bottom out in were per-element\n  \
         closures returning `Option` (unvectorizable); they are now a branch-free dense pass over the\n  \
         contiguous value slice with a word-at-a-time validity AND, so larger-N leaf throughput rises\n  \
         (see the leaf rows above). A nested op adds its container rebuild (offsets / field schema +\n  \
         a per-row validity pass) on top of that leaf work."
    );
}
