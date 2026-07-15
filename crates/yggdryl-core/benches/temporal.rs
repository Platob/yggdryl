//! Time **and** memory for the temporal surface: the civil-calendar math (`Date`↔`(y,m,d)`), the
//! timestamp wall-clock decomposition (naive/UTC vs a DST-aware IANA zone, which consults the tz
//! database), unit conversion, and the byte codec. Focus: that the calendar math and the
//! naive/UTC decomposition are stack-only, and that the IANA-zone path (its one expensive step)
//! stays allocation-free.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench temporal`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::temporal::{Date32, Duration64, Time64, TimeUnit, Ts64, Tz};

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

fn measure(iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = f64::from(iters);
    (
        total / secs / 1_000_000.0,
        (a1 - a0) as f64 / total,
        (b1 - b0) as f64 / total,
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<44} {mops:8.2}      {allocs:7.3}      {bytes:8.1}");
}

fn main() {
    let iters = 200_000;
    println!("io::fixed::temporal — time & memory ({iters} iters)\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(82));

    let date = Date32::from_ymd(2024, 2, 29).unwrap();
    row(
        "Date32::from_ymd (calendar math)",
        measure(iters, || {
            let _ = Date32::from_ymd(2024, 2, 29).unwrap();
        }),
    );
    row(
        "Date32::to_ymd (calendar math)",
        measure(iters, || {
            let _ = date.to_ymd();
        }),
    );

    let utc = Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    let paris = utc.with_timezone(Tz::europe_paris());
    row(
        "Ts64::to_datetime (UTC)",
        measure(iters, || {
            let _ = utc.to_datetime();
        }),
    );
    row(
        "Ts64::to_datetime (IANA, DST lookup)",
        measure(iters, || {
            let _ = paris.to_datetime();
        }),
    );
    row(
        "Ts64::from_datetime (UTC)",
        measure(iters, || {
            let _ =
                Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
        }),
    );
    row(
        "Ts64::to_unit (s -> ms)",
        measure(iters, || {
            let _ = utc.to_unit(TimeUnit::Millisecond).unwrap();
        }),
    );

    row(
        "Ts64 serialize+deserialize (zoned)",
        measure(iters, || {
            let bytes = paris.serialize_bytes();
            let _ = Ts64::deserialize_bytes(&bytes).unwrap();
        }),
    );

    let (a, b) = (Duration64::seconds(1), Duration64::milliseconds(500));
    row(
        "Duration64::checked_add (unit align)",
        measure(iters, || {
            let _ = a.checked_add(&b).unwrap();
        }),
    );

    // Cross-type converters (the "any temporal -> any temporal" matrix).
    let time = Time64::from_hms_nano(13, 45, 30, 0).unwrap();
    row(
        "Date32::at_time -> Ts64",
        measure(iters, || {
            let _ = date.at_time(&time, TimeUnit::Second, Tz::UTC).unwrap();
        }),
    );
    row(
        "Ts64::to_date (extract)",
        measure(iters, || {
            let _ = utc.to_date().unwrap();
        }),
    );
    row(
        "Ts64::to_duration (span)",
        measure(iters, || {
            let _ = utc.to_duration().unwrap();
        }),
    );
    let span = Duration64::seconds(86_400);
    row(
        "Duration64::to_timestamp",
        measure(iters, || {
            let _ = span.to_timestamp(Tz::UTC).unwrap();
        }),
    );

    // The one expensive step — an IANA-zone offset lookup (binary search in the tz database),
    // exercised directly; must stay allocation-free.
    row(
        "Tz::offset_seconds_at (IANA DST lookup)",
        measure(iters, || {
            let _ = Tz::europe_paris().offset_seconds_at(1_721_040_000);
        }),
    );

    // The flexible duration parser (compound / clock / ISO forms).
    row(
        "Duration64::parse_str (\"1h30m15s\")",
        measure(iters, || {
            let _ = Duration64::parse_str("1h30m15s").unwrap();
        }),
    );

    // The columnar Arrow interop (feature `arrow`): a native `ts64`, the widened `ts32`, and the
    // `FixedSizeBinary(12)` `ts96` — build / export / import / codec over a 4096-element column. The
    // story is `allocs/op` + `bytes/op`: the native export/import share the payload (near-zero,
    // regardless of the 4096 elements), the widen path pays one `len*8` i64 buffer, and `ts96` byte
    // data shares too.
    #[cfg(feature = "arrow")]
    columnar_arrow();
}

/// The columnar Arrow-interop rows (feature `arrow`) — split out so the value-type benchmark above
/// stays runnable without the feature (mirrors `benches/decimal.rs`).
#[cfg(feature = "arrow")]
fn columnar_arrow() {
    use yggdryl_core::io::fixed::temporal::{Ts32, Ts96};
    use yggdryl_core::io::fixed::{Ts32Serie, Ts64Serie, Ts96Serie};

    let iters = 20_000u32;
    let n = 4096usize;

    let ts64: Vec<Option<Ts64>> = (0..n as i64)
        .map(|i| {
            (i % 4 != 0).then(|| Ts64::from_epoch(i as i128, TimeUnit::Second, Tz::UTC).unwrap())
        })
        .collect();
    let ts64_col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &ts64).unwrap();
    let ts64_field = ts64_col.to_field("t").to_arrow();
    let ts64_array = ts64_col.to_arrow_array().unwrap();
    let ts64_bytes = ts64_col.serialize_bytes();

    let ts32: Vec<Ts32> = (0..n as i32)
        .map(|i| Ts32::from_epoch(i as i128, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let ts32_col = Ts32Serie::from_values(TimeUnit::Second, Tz::UTC, &ts32).unwrap();
    let ts32_field = ts32_col.to_field("t").to_arrow();
    let ts32_array = ts32_col.to_arrow_array().unwrap();

    let ts96: Vec<Ts96> = (0..n as i128)
        .map(|i| Ts96::from_epoch(i * 1_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap())
        .collect();
    let ts96_col = Ts96Serie::from_values(TimeUnit::Nanosecond, Tz::UTC, &ts96).unwrap();
    let ts96_field = ts96_col.to_field("t").to_arrow();
    let ts96_array = ts96_col.to_arrow_array().unwrap();

    println!("\nio::fixed::temporal columnar Arrow interop ({iters} iters, {n}-element column)\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(82));

    row(
        "Ts64Serie::from_options (build)",
        measure(iters, || {
            let _ = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &ts64).unwrap();
        }),
    );
    row(
        "Ts64Serie::to_arrow_array (native, share)",
        measure(iters, || {
            let _ = ts64_col.to_arrow_array().unwrap();
        }),
    );
    row(
        "Ts64Serie::from_arrow_array (share payload)",
        measure(iters, || {
            let _ = Ts64Serie::from_arrow_array(ts64_array.as_ref(), &ts64_field).unwrap();
        }),
    );
    row(
        "Ts64Serie serialize_bytes",
        measure(iters, || {
            let _ = ts64_col.serialize_bytes();
        }),
    );
    row(
        "Ts64Serie deserialize_bytes",
        measure(iters, || {
            let _ = Ts64Serie::deserialize_bytes(&ts64_bytes).unwrap();
        }),
    );

    row(
        "Ts32Serie::to_arrow_array (widen i32->i64)",
        measure(iters, || {
            let _ = ts32_col.to_arrow_array().unwrap();
        }),
    );
    row(
        "Ts32Serie::from_arrow_array (narrow)",
        measure(iters, || {
            let _ = Ts32Serie::from_arrow_array(ts32_array.as_ref(), &ts32_field).unwrap();
        }),
    );

    row(
        "Ts96Serie::to_arrow_array (FSB12, share)",
        measure(iters, || {
            let _ = ts96_col.to_arrow_array().unwrap();
        }),
    );
    row(
        "Ts96Serie::from_arrow_array (FSB12 share)",
        measure(iters, || {
            let _ = Ts96Serie::from_arrow_array(ts96_array.as_ref(), &ts96_field).unwrap();
        }),
    );
}
