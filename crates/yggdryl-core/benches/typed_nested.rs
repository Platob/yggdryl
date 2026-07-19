//! Time **and** memory benchmark for the [`nested`](yggdryl_core::typed::nested) typed layer — the
//! erased [`Column`] carrier and the recursive struct / list / map "tables"
//! ([`StructSerie`](yggdryl_core::typed::StructSerie),
//! [`ListSerie`](yggdryl_core::typed::ListSerie), [`MapSerie`](yggdryl_core::typed::MapSerie)). It
//! shows the shape of the layout costs: a **build** owns one buffer per leaf column plus the
//! offsets / validity of each nested level; a **`column_by_name`** lookup is a pure borrow (zero
//! allocation); a **`row(i)`** / **`list(i)`** / **`get(i)`** materializes an owned scalar row; and a
//! deep **`column_by_name_mut` + `set`** sweep rewrites a child in place with no per-row allocation.
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the other
//! benches. Run with `cargo bench -p yggdryl-core --bench typed_nested`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::typed::fixedbyte::Int64;
use yggdryl_core::typed::varbyte::Utf8;
use yggdryl_core::typed::{Column, FixedSerie, ListSerie, MapSerie, StructSerie, VarSerie};

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

/// A tiny deterministic LCG so the "random access" rows sweep indices without a `rand` dependency.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, modulo: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 33) as usize) % modulo
    }
}

fn main() {
    let iters = 2_000;
    let n = 100_000; // rows

    // Source values for the leaf columns — pre-built so a build row measures the column build, not the
    // integer range or the string formatting.
    let ids: Vec<i64> = (0..n as i64).collect();
    let names: Vec<String> = (0..n).map(|i| format!("u{i}")).collect();
    let amounts: Vec<i64> = (0..n as i64).map(|v| v * 7).collect();
    let tags: Vec<String> = (0..n).map(|i| format!("t{}", i % 97)).collect();

    // A 3-column table: `id` (Int64) + `name` (Utf8) + a nested `address` Struct child (Int64 `amount`
    // + Utf8 `tag`). Rebuilt fresh each iteration so the build row owns the whole allocation cost.
    let build = || {
        let id = FixedSerie::<Int64>::from_values(&ids).with_name("id");
        let name = VarSerie::<Utf8>::from_values(&names).with_name("name");
        let amount = FixedSerie::<Int64>::from_values(&amounts).with_name("amount");
        let tag = VarSerie::<Utf8>::from_values(&tags).with_name("tag");
        let address = StructSerie::from_columns(vec![Column::from(amount), Column::from(tag)])
            .unwrap()
            .with_name("address");
        StructSerie::from_columns(vec![
            Column::from(id),
            Column::from(name),
            Column::from(address),
        ])
        .unwrap()
        .with_name("people")
    };

    println!("typed nested — time & memory ({iters} iters over {n} rows)\n");
    println!(
        "  {:<44} {:>8}    {:>8}    {:>11}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(84));

    // -- StructSerie: build, borrow lookup, row materialize, deep mutation ------------------------
    println!("  -- struct \"table\" (3 cols, 1 nested) --");
    row(
        "StructSerie build (from_columns, nested)",
        measure(n, iters / 4, || {
            black_box(build());
        }),
    );

    let table = build();
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
    // A borrow lookup: walk the child list and compare names — no allocation.
    row(
        "column_by_name (borrow lookup)",
        measure(1, iters * 64, || {
            black_box(table.column_by_name(black_box("address")));
        }),
    );
    // A row materialize at a random index — owns the row's names + erased values (nested row nests).
    row(
        "row(i) (random, materialize)",
        measure(1, iters * 8, || {
            let i = rng.next(n);
            black_box(table.row(black_box(i)));
        }),
    );

    // A deep mutation sweep: recover the concrete leaf behind the erased `&mut Column` and rewrite
    // every row in place (one positioned write per row, no per-row allocation).
    let mut table_mut = build();
    row(
        "column_by_name_mut + set (deep sweep)",
        measure(n, iters / 4, || {
            if let Some(Column::Int64(serie)) = table_mut.column_by_name_mut("id") {
                for i in 0..n {
                    serie.set(i, black_box(i as i64 + 1)).unwrap();
                }
            }
        }),
    );

    // -- ListSerie: build (offsets + flattened child) + list(i) materialize ----------------------
    println!("\n  -- list column (i32 offsets + flattened child) --");
    let stride = 4; // elements per list
    let num_lists = n / stride;
    let build_list = || {
        let child = FixedSerie::<Int64>::from_values(&ids);
        let mut list = ListSerie::new("nums", Column::from(child));
        for _ in 0..num_lists {
            list.push(stride);
        }
        list
    };
    row(
        "ListSerie build (push demarcation)",
        measure(num_lists, iters / 4, || {
            black_box(build_list());
        }),
    );
    let list = build_list();
    let mut rng = Lcg(0x1234_5678_9ABC_DEF0);
    row(
        "list(i) (random, materialize)",
        measure(1, iters * 8, || {
            let i = rng.next(num_lists);
            black_box(list.list(black_box(i)));
        }),
    );

    // -- MapSerie: build (offsets + key/value entries struct) + get(i) materialize ----------------
    println!("\n  -- map column (i32 offsets + key/value entries) --");
    let num_maps = n / stride;
    let build_map = || {
        let keys = VarSerie::<Utf8>::from_values(&names);
        let vals = FixedSerie::<Int64>::from_values(&ids);
        let mut map = MapSerie::new("m", Column::from(keys), Column::from(vals)).unwrap();
        for _ in 0..num_maps {
            map.push(stride);
        }
        map
    };
    row(
        "MapSerie build (push demarcation)",
        measure(num_maps, iters / 4, || {
            black_box(build_map());
        }),
    );
    let map = build_map();
    let mut rng = Lcg(0x0F0F_0F0F_0F0F_0F0F);
    row(
        "get(i) (random, materialize)",
        measure(1, iters * 8, || {
            let i = rng.next(num_maps);
            black_box(map.get(black_box(i)));
        }),
    );

    // Keep the built tables alive so nothing is dropped inside a measured window.
    black_box((&table, &table_mut, &list, &map));
}
