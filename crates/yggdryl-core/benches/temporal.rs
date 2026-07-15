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
}
