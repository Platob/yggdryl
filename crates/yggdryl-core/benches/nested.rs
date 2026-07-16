//! Time **and** memory benchmark for the `io::nested` struct layer: erasing a typed column into a
//! `Box<dyn AnySerie>` (via [`boxed`]), assembling a [`StructSerie`], reading rows, and the
//! byte-codec round-trip. A struct column borrows its children (each an erased `AnySerie`), so the
//! allocations/op column is the story — navigation is free, a row is a bounded per-cell copy.
//!
//! Dependency-free harness (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench nested`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnySerie};

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
        (a1 - a0) as f64 / total,
        (b1 - b0) as f64 / total,
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<40} {mops:8.2}      {allocs:6.2}      {bytes:8.1}");
}

fn build_table(n: usize) -> StructSerie {
    let ids = boxed(Serie::from_values(&(0..n as i64).collect::<Vec<_>>()));
    let names = boxed(Utf8Serie::from_strs(
        &(0..n).map(|_| Some("value")).collect::<Vec<_>>(),
    ));
    StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap()
}

/// A `list<i32>` of `n` rows, each 2 elements over a flat child of `2n` i32.
fn build_list(n: usize) -> ListSerie {
    let flat: Vec<i32> = (0..(n * 2) as i32).collect();
    let offsets: Vec<i32> = (0..=n).map(|i| (i * 2) as i32).collect();
    ListSerie::from_values(Serie::from_values(&flat).named("item"), &offsets, None).unwrap()
}

/// A `map<utf8, i64>` of `n` rows, each 2 entries over `2n` flat entries (keys are `keys`).
fn build_map(n: usize, keys: &[String]) -> MapSerie {
    let key_refs: Vec<Option<&str>> = keys.iter().map(|s| Some(s.as_str())).collect();
    let vals: Vec<i64> = (0..(n * 2) as i64).collect();
    let offsets: Vec<i32> = (0..=n).map(|i| (i * 2) as i32).collect();
    MapSerie::from_entries(
        Utf8Serie::from_strs(&key_refs).named("key"),
        Serie::from_values(&vals).named("value"),
        &offsets,
        None,
        false,
    )
    .unwrap()
}

/// `n` short key strings `k0..k{n}` — kept alive so `Utf8Serie::from_strs` can borrow them.
fn key_strings(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("k{i}")).collect()
}

/// A `list<map<utf8, struct<{a: i32, b: list<i32>}>>>` of 1 outer row over `entries` map entries —
/// the four-level-deep column for the depth-scaling row.
fn build_deep(entries: usize) -> ListSerie {
    let b = {
        let flat: Vec<i32> = (0..(entries * 2) as i32).collect();
        let offsets: Vec<i32> = (0..=entries).map(|i| (i * 2) as i32).collect();
        ListSerie::from_values(Serie::from_values(&flat).named("item"), &offsets, None).unwrap()
    };
    let a = Serie::from_values(&(0..entries as i32).collect::<Vec<_>>());
    let structs = StructSerie::from_series(vec![a.named("a"), b.named("b")]).unwrap();
    let keys = key_strings(entries);
    let key_refs: Vec<Option<&str>> = keys.iter().map(|s| Some(s.as_str())).collect();
    let maps = MapSerie::from_entries(
        Utf8Serie::from_strs(&key_refs).named("key"),
        structs.named("value"),
        &[0, entries as i32],
        None,
        false,
    )
    .unwrap();
    ListSerie::from_values(maps.named("item"), &[0, 1], None).unwrap()
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;
    let ids: Vec<i64> = (0..n as i64).collect();
    let names: Vec<Option<&str>> = (0..n).map(|_| Some("value")).collect();
    let table = build_table(n);
    let frame = table.serialize_bytes();

    println!("Nested struct — time & memory ({iters} iters, {n} rows × 2 cols)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));

    row(
        "boxed(Serie<i64>) (erase)",
        measure(1, iters, || {
            let _ = boxed(Serie::from_values(&ids));
        }),
    );
    row(
        "boxed(Utf8Serie) (erase)",
        measure(1, iters, || {
            let _ = boxed(Utf8Serie::from_strs(&names));
        }),
    );
    row(
        "StructSerie::from_named (2 cols)",
        measure(1, iters, || {
            let _ = build_table(n);
        }),
    );
    row(
        "StructSerie::column + type_id (navigate)",
        measure(2, iters, || {
            let _ = table.column(0).map(|c| c.type_id());
            let _ = table.column_named("name").map(|c| c.type_id());
        }),
    );
    row(
        "StructSerie::row",
        measure(1, iters, || {
            let _ = table.row(n / 2);
        }),
    );
    row(
        "StructSerie::serialize_bytes",
        measure(1, iters, || {
            let _ = table.serialize_bytes();
        }),
    );
    row(
        "StructSerie::deserialize_bytes",
        measure(1, iters, || {
            let _ = StructSerie::deserialize_bytes(&frame).unwrap();
        }),
    );

    // ---------------------------------------------------------------------------------
    // ListSerie — build, navigate, byte codec, slice (n rows x 2 elements)
    // ---------------------------------------------------------------------------------
    let list = build_list(n);
    let list_frame = list.serialize_bytes();
    println!("\nNested list<i32> — time & memory ({iters} iters, {n} rows x 2 elems)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));
    row(
        "ListSerie::from_values (build)",
        measure(1, iters, || {
            let _ = build_list(n);
        }),
    );
    row(
        "ListSerie::row (navigate)",
        measure(1, iters, || {
            let _ = list.row(n / 2);
        }),
    );
    row(
        "ListSerie::row_scalar (navigate)",
        measure(1, iters, || {
            let _ = list.row_scalar(n / 2);
        }),
    );
    row(
        "ListSerie::serialize_bytes",
        measure(1, iters, || {
            let _ = list.serialize_bytes();
        }),
    );
    row(
        "ListSerie::deserialize_bytes",
        measure(1, iters, || {
            let _ = ListSerie::deserialize_bytes(&list_frame).unwrap();
        }),
    );
    row(
        "ListSerie::slice(n/4, n/2)",
        measure(1, iters, || {
            let _ = list.slice(n / 4, n / 2);
        }),
    );

    // ---------------------------------------------------------------------------------
    // MapSerie — build, navigate, get_value scan, byte codec, slice
    // ---------------------------------------------------------------------------------
    let keys = key_strings(n * 2);
    let map = build_map(n, &keys);
    let map_frame = map.serialize_bytes();
    // A single wide row of `scan` entries, to isolate the `get_value` per-key scan cost.
    let scan = 32usize;
    let scan_keys = key_strings(scan);
    let scan_map = build_map_single_row(&scan_keys);
    let present = scan_map.keys().value(scan - 1); // last key -> full scan, present
    let absent = boxed(Utf8Serie::from_strs(&[Some("absent")])).value(0); // full scan, miss
    println!("\nNested map<utf8, i64> — time & memory ({iters} iters, {n} rows x 2 entries)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));
    row(
        "MapSerie::from_entries (build)",
        measure(1, iters, || {
            let _ = build_map(n, &keys);
        }),
    );
    row(
        "MapSerie::keys/values/entries (navigate)",
        measure(3, iters, || {
            let _ = map.keys().len();
            let _ = map.values().len();
            let _ = map.entries().num_columns();
        }),
    );
    row(
        "MapSerie::get_value scan (32, present)",
        measure(1, iters, || {
            let _ = scan_map.get_value(0, &present);
        }),
    );
    row(
        "MapSerie::get_value scan (32, absent)",
        measure(1, iters, || {
            let _ = scan_map.get_value(0, &absent);
        }),
    );
    row(
        "MapSerie::row (navigate)",
        measure(1, iters, || {
            let _ = map.row(n / 2);
        }),
    );
    row(
        "MapSerie::serialize_bytes",
        measure(1, iters, || {
            let _ = map.serialize_bytes();
        }),
    );
    row(
        "MapSerie::deserialize_bytes",
        measure(1, iters, || {
            let _ = MapSerie::deserialize_bytes(&map_frame).unwrap();
        }),
    );
    row(
        "MapSerie::slice(n/4, n/2)",
        measure(1, iters, || {
            let _ = map.slice(n / 4, n / 2);
        }),
    );

    // ---------------------------------------------------------------------------------
    // Depth scaling — a 4-level list<map<utf8, struct<{a, b:list<i32>}>>> build + serialize
    // ---------------------------------------------------------------------------------
    let depth_iters = 5_000;
    let depth_entries = 256usize;
    let deep = build_deep(depth_entries);
    let deep_frame = deep.serialize_bytes();
    println!(
        "\n4-level nested (list<map<utf8, struct<{{a, b:list<i32>}}>>>) — {depth_iters} iters, \
         {depth_entries} innermost entries\n"
    );
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));
    row(
        "build (4 levels)",
        measure(1, depth_iters, || {
            let _ = build_deep(depth_entries);
        }),
    );
    row(
        "serialize_bytes (4 levels)",
        measure(1, depth_iters, || {
            let _ = deep.serialize_bytes();
        }),
    );
    row(
        "deserialize_bytes (4 levels)",
        measure(1, depth_iters, || {
            let _ = ListSerie::deserialize_bytes(&deep_frame).unwrap();
        }),
    );

    #[cfg(feature = "arrow")]
    arrow_section(iters, n, &list, &map);
}

/// A single-row `map<utf8, i64>` over `keys.len()` entries — the wide row the `get_value` scan
/// benchmark probes.
fn build_map_single_row(keys: &[String]) -> MapSerie {
    let key_refs: Vec<Option<&str>> = keys.iter().map(|s| Some(s.as_str())).collect();
    let vals: Vec<i64> = (0..keys.len() as i64).collect();
    MapSerie::from_entries(
        Utf8Serie::from_strs(&key_refs).named("key"),
        Serie::from_values(&vals).named("value"),
        &[0, keys.len() as i32],
        None,
        false,
    )
    .unwrap()
}

/// The Arrow interop rows for the nested list / map columns (feature `arrow`): `to_arrow_array` /
/// `from_arrow_array`, and a note on whether the primitive child buffer is Arc-shared (zero-copy).
#[cfg(feature = "arrow")]
fn arrow_section(iters: u32, n: usize, list: &ListSerie, map: &MapSerie) {
    use arrow_array::Array;

    let list_field = list.to_field("l").to_arrow_field();
    let list_array = list.to_arrow_array().unwrap();
    let map_field = map.to_field("m").to_arrow_field();
    let map_array = map.to_arrow_array().unwrap();

    // Whether the exported list child shares the Serie's Arc value buffer (a pointer bump, not a
    // copy): the child column's own direct Arrow export and the child inside the exported ListArray
    // must point at the same allocation.
    let direct_ptr = list.values().to_arrow_array().unwrap().to_data().buffers()[0].as_ptr();
    let child_ptr = list_array.values().to_data().buffers()[0].as_ptr();
    let shared = direct_ptr == child_ptr;

    println!("\nNested list / map <-> Arrow ({iters} iters, {n} rows)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));
    row(
        "ListSerie::to_arrow_array",
        measure(1, iters, || {
            let _ = list.to_arrow_array().unwrap();
        }),
    );
    row(
        "ListSerie::from_arrow_array",
        measure(1, iters, || {
            let _ = ListSerie::from_arrow_array(&list_array, &list_field).unwrap();
        }),
    );
    row(
        "MapSerie::to_arrow_array",
        measure(1, iters, || {
            let _ = map.to_arrow_array().unwrap();
        }),
    );
    row(
        "MapSerie::from_arrow_array",
        measure(1, iters, || {
            let _ = MapSerie::from_arrow_array(&map_array, &map_field).unwrap();
        }),
    );
    println!(
        "\n  list<i32> child value buffer is Arc-shared on export (zero-copy): {}",
        if shared { "yes" } else { "no" }
    );
}
