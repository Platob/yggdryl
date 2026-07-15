//! Deterministic allocation budgets for the temporal Arrow interop (feature `arrow`) — the
//! zero-copy claims asserted directly through a counting global allocator (optimizer- and
//! machine-independent). The story is the **payload**: for a `len`-element column the raw counts
//! are `len * WIDTH` bytes, and the interop must never copy them on the native path.
//!
//! - **Export (native `ts64`)** allocates only the null bitmap (`len/8`) — never the `len*8`
//!   payload, which is shared as an `Arc`.
//! - **Export (widened `ts32` / `duration32`)** materializes exactly one fresh `i64` values buffer
//!   (`len*8`), since Arrow has no 32-bit temporal type.
//! - **Import (dense native `ts64`)** Arc-shares the values buffer — **zero** payload allocation on
//!   the fast path (mirrors `DecimalSerie::from_arrow_array`); the copy-and-canonicalize slow path
//!   (garbage under a null) is what a payload-sized allocation looks like, for contrast.
//!
//! Its own test binary with its own counting global allocator, holding a single `#[test]`. Bytes
//! (not just counts) are compared, since the whole point is a `len`-scaled payload vs a tiny mask.
#![cfg(feature = "arrow")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use arrow_array::types::TimestampSecondType;
use arrow_array::PrimitiveArray;
use arrow_buffer::{NullBuffer, ScalarBuffer};

use yggdryl_core::io::fixed::temporal::{Duration32, TimeUnit, Ts32, Ts64, Tz};
use yggdryl_core::io::fixed::{Duration32Serie, Ts32Serie, Ts64Serie};

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

/// The number of bytes allocated per `op` call, averaged over `iters` runs (a one-shot warm-up
/// first, so a lazily-built cache is not charged to the measured window).
fn bytes_per_op(iters: usize, mut op: impl FnMut()) -> usize {
    op();
    let before = BYTES.load(Relaxed);
    for _ in 0..iters {
        op();
    }
    (BYTES.load(Relaxed) - before) / iters
}

#[test]
fn allocation_budgets() {
    let iters = 500;
    let n = 4096usize;
    let payload = n * 8; // the raw counts for a native ts64 column: 32 KiB

    // Build the fixtures (outside the measured windows).
    let nullable: Vec<Option<Ts64>> = (0..n as i64)
        .map(|i| {
            (i % 4 != 0).then(|| Ts64::from_epoch(i as i128, TimeUnit::Second, Tz::UTC).unwrap())
        })
        .collect();
    let ts64_nullable = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &nullable).unwrap();
    let dense: Vec<Ts64> = (0..n as i64)
        .map(|i| Ts64::from_epoch(i as i128, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let ts64_dense = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &dense).unwrap();
    let ts32_vals: Vec<Ts32> = (0..n as i32)
        .map(|i| Ts32::from_epoch(i as i128, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let ts32_dense = Ts32Serie::from_values(TimeUnit::Second, Tz::UTC, &ts32_vals).unwrap();
    let dur32_vals: Vec<Duration32> = (0..n as i32).map(Duration32::milliseconds).collect();
    let dur32_dense =
        Duration32Serie::from_values(TimeUnit::Millisecond, Tz::NAIVE, &dur32_vals).unwrap();

    let field = ts64_dense.to_field("t").to_arrow();
    let dense_array = ts64_dense.to_arrow_array().unwrap();

    // A foreign ts64 array with non-zero bytes UNDER a null slot — the copy-and-canonicalize slow
    // path (it cannot share, since the shared buffer would leak the garbage into identity).
    let mut raw: Vec<i64> = (0..n as i64).collect();
    raw[7] = 0x7fff_ffff_ffff;
    let valid: Vec<bool> = (0..n).map(|i| i != 7).collect();
    let garbage_array = PrimitiveArray::<TimestampSecondType>::new(
        ScalarBuffer::from(raw),
        Some(NullBuffer::from(valid)),
    );

    // ---- Export: native ts64 allocates only the null bitmap, not the payload -----------------
    let native_export = bytes_per_op(iters, || {
        let _ = black_box(ts64_nullable.to_arrow_array().unwrap());
    });
    assert!(
        native_export < payload / 4,
        "native ts64 export must allocate only the null bitmap (~{} B), not the {payload}-byte \
         payload — got {native_export} B/op",
        n / 8
    );
    let dense_export = bytes_per_op(iters, || {
        let _ = black_box(ts64_dense.to_arrow_array().unwrap());
    });
    assert!(
        dense_export < payload / 4,
        "dense ts64 export shares its payload (no null bitmap) — got {dense_export} B/op"
    );

    // ---- Export: widen ts32 / duration32 materializes exactly one values buffer --------------
    let ts32_export = bytes_per_op(iters, || {
        let _ = black_box(ts32_dense.to_arrow_array().unwrap());
    });
    assert!(
        ts32_export >= payload && ts32_export < payload + payload / 8,
        "ts32 widen must allocate exactly one i64 values buffer (~{payload} B) — got {ts32_export} B/op"
    );
    let dur32_export = bytes_per_op(iters, || {
        let _ = black_box(dur32_dense.to_arrow_array().unwrap());
    });
    assert!(
        dur32_export >= payload && dur32_export < payload + payload / 8,
        "duration32 widen must allocate exactly one i64 values buffer (~{payload} B) — got {dur32_export} B/op"
    );

    // ---- Import: dense native ts64 Arc-shares — zero payload allocation on the fast path ------
    let dense_import = bytes_per_op(iters, || {
        let _ = black_box(Ts64Serie::from_arrow_array(dense_array.as_ref(), &field).unwrap());
    });
    assert!(
        dense_import < payload / 64,
        "dense ts64 import must Arc-share the payload (zero copy) — got {dense_import} B/op vs a \
         {payload}-byte payload"
    );

    // Contrast: the slow path (garbage under a null) DOES copy the whole payload — proving the fast
    // path's saving is the full `len*WIDTH`, not a rounding artifact.
    let garbage_import = bytes_per_op(iters, || {
        let _ = black_box(Ts64Serie::from_arrow_array(&garbage_array, &field).unwrap());
    });
    assert!(
        garbage_import >= payload,
        "the garbage-under-null slow path copies the payload (~{payload} B) — got {garbage_import} B/op"
    );
    assert!(
        dense_import * 100 < garbage_import,
        "the fast path ({dense_import} B/op) must allocate far less than the copying slow path \
         ({garbage_import} B/op)"
    );

    // Sanity: the measured paths produce correct columns (so the optimizer cannot have elided them).
    assert_eq!(
        Ts64Serie::from_arrow_array(dense_array.as_ref(), &field).unwrap(),
        ts64_dense
    );
    assert_eq!(
        Ts64Serie::from_arrow_array(&garbage_array, &field)
            .unwrap()
            .null_count(),
        1
    );
}
