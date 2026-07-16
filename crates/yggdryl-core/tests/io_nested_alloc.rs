//! Deterministic allocation budgets for the `io::nested` layer. Allocation counts are optimizer-
//! and machine-independent, so they assert the zero-copy claims directly: navigating a struct
//! column — borrowing a child `AnySerie`, reading its `len` / `type_id` / null count, looking a
//! field up by name — touches **no** heap (the data lives in the columns; navigation is borrows and
//! integer reads).
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnySerie};

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

    // ---- list / map navigation is zero-copy (borrow offsets + children) -----------------

    let list = ListSerie::from_values(
        Serie::from_values(&[1i32, 2, 3, 4, 5]).named("item"),
        &[0, 2, 2, 5],
        None,
    )
    .unwrap();
    let list_nav = allocs_over(iters, || {
        let _ = list.offsets();
        let _ = list.values().len();
        let _ = list.item_field();
        let _ = list.value_range(1);
        let _ = list.len();
        let _ = list.null_count();
    });
    assert_eq!(
        list_nav, 0,
        "ListSerie navigation must be zero-copy (got {list_nav})"
    );

    let map = MapSerie::from_entries(
        Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key"),
        Serie::from_values(&[1i64, 2, 3]).named("value"),
        &[0, 2, 2, 3],
        None,
        false,
    )
    .unwrap();
    let map_nav = allocs_over(iters, || {
        let _ = map.offsets();
        let _ = map.keys().len();
        let _ = map.values().len();
        let _ = map.entries().num_columns();
        let _ = map.key_field();
        let _ = map.value_field();
        let _ = map.value_range(0);
    });
    assert_eq!(
        map_nav, 0,
        "MapSerie navigation must be zero-copy (got {map_nav})"
    );

    // ---- MapSerie::get_value: the per-key scan is allocation-free (the optimization) ----

    // A single row of eight entries; the last value is NULL. Probes are built ONCE, outside the
    // measured loop, so only the scan (and any returned cell) is counted.
    let scan_map = MapSerie::from_entries(
        Utf8Serie::from_strs(&[
            Some("k0"),
            Some("k1"),
            Some("k2"),
            Some("k3"),
            Some("k4"),
            Some("k5"),
            Some("k6"),
            Some("k7"),
        ])
        .named("key"),
        Serie::from_options(&[
            Some(0i64),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
            None,
        ])
        .named("value"),
        &[0, 8],
        None,
        false,
    )
    .unwrap();
    let first_key = scan_map.keys().value(0); // present, non-null value (matches immediately)
    let last_key = scan_map.keys().value(7); // present, NULL value (forces a full scan)
    let absent_key = boxed(Utf8Serie::from_strs(&[Some("zzz")])).value(0); // absent (full scan)

    // An absent-key lookup scans all eight stored keys and allocates NOTHING. Before this
    // optimization each compared key materialized one owned `AnyScalar` (a fresh bytes `Vec`) — a
    // full scan of this row was eight allocations; the borrowed/stack-scratch compare makes it zero.
    let getv_absent = allocs_over(iters, || {
        let _ = scan_map.get_value(0, &absent_key);
    });
    assert_eq!(
        getv_absent, 0,
        "get_value's key scan must be allocation-free (absent key over 8 entries, got {getv_absent})"
    );

    // A present key mapping to a NULL value is fully allocation-free: the full scan plus a null
    // result cell touch no heap.
    let getv_present_null = allocs_over(iters, || {
        let _ = scan_map.get_value(0, &last_key);
    });
    assert_eq!(
        getv_present_null, 0,
        "get_value on a present key (null value) must be allocation-free (got {getv_present_null})"
    );

    // A present key with a non-null value allocates only the single returned value cell — at most
    // one allocation per lookup (so <= `iters` total), independent of how many keys the scan
    // compared. (`allocs_over` sums allocations across all `iters` iterations.)
    let getv_present_int = allocs_over(iters, || {
        let _ = scan_map.get_value(0, &first_key);
    });
    assert!(
        getv_present_int <= iters,
        "get_value allocates at most the single returned value cell per lookup \
         (got {getv_present_int} over {iters} lookups)"
    );

    // ---- from_values / from_entries build a bounded constant, not per-row -----------------

    // The offset count differs 512x between the two builds, yet the ALLOCATION COUNT is identical —
    // `from_values` allocates the offsets `Vec` (one allocation) plus a small constant, never per row.
    let offsets_small: Vec<i32> = vec![0; 3]; // 2 rows
    let offsets_big: Vec<i32> = vec![0; 1025]; // 1024 rows
    let build_list_small = allocs_over(iters, || {
        let items = Serie::<i32>::from_values(&[]).named("item");
        let _ = ListSerie::from_values(items, &offsets_small, None).unwrap();
    });
    let build_list_big = allocs_over(iters, || {
        let items = Serie::<i32>::from_values(&[]).named("item");
        let _ = ListSerie::from_values(items, &offsets_big, None).unwrap();
    });
    assert_eq!(
        build_list_small, build_list_big,
        "ListSerie::from_values must allocate a bounded constant, not per-row \
         ({build_list_small} vs {build_list_big})"
    );

    let build_map_small = allocs_over(iters, || {
        let k = Utf8Serie::from_strs(&[]).named("key");
        let v = Serie::<i64>::from_values(&[]).named("value");
        let _ = MapSerie::from_entries(k, v, &offsets_small, None, false).unwrap();
    });
    let build_map_big = allocs_over(iters, || {
        let k = Utf8Serie::from_strs(&[]).named("key");
        let v = Serie::<i64>::from_values(&[]).named("value");
        let _ = MapSerie::from_entries(k, v, &offsets_big, None, false).unwrap();
    });
    assert_eq!(
        build_map_small, build_map_big,
        "MapSerie::from_entries must allocate a bounded constant, not per-row \
         ({build_map_small} vs {build_map_big})"
    );

    // ---- Arrow export shares the primitive child's Arc value buffer (zero-copy) -----------

    #[cfg(feature = "arrow")]
    {
        use arrow_array::Array;
        let values: Vec<i32> = (0..1024).collect();

        // A list's primitive child exports with a shared Arc value buffer (a pointer bump, not a copy).
        let serie = Serie::from_values(&values);
        let direct_ptr = serie.to_arrow_array().to_data().buffers()[0].as_ptr();
        let list = ListSerie::from_values(serie.named("item"), &[0, 512, 1024], None).unwrap();
        let list_array = list.to_arrow_array().unwrap();
        let child_ptr = list_array.values().to_data().buffers()[0].as_ptr();
        assert_eq!(
            direct_ptr, child_ptr,
            "list child Arrow export copied the value buffer"
        );

        // A map's primitive VALUE child shares its Arc value buffer through the MapArray export.
        let keys: Vec<Option<&str>> = (0..1024).map(|_| Some("k")).collect();
        let vserie = Serie::from_values(&values);
        let value_ptr = vserie.to_arrow_array().to_data().buffers()[0].as_ptr();
        let map = MapSerie::from_entries(
            Utf8Serie::from_strs(&keys).named("key"),
            vserie.named("value"),
            &[0, 512, 1024],
            None,
            false,
        )
        .unwrap();
        let map_array = map.to_arrow_array().unwrap();
        let value_child_ptr = map_array.entries().column(1).to_data().buffers()[0].as_ptr();
        assert_eq!(
            value_ptr, value_child_ptr,
            "map value child Arrow export copied the value buffer"
        );
    }
}
