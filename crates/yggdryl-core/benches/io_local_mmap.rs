//! Time **and** memory benchmark for the memory-mapped
//! [`Mmap`](yggdryl_core::io::local::Mmap) source: open/create, page-sized mapped reads and
//! writes through the shared `IOBase` surface, auto-resizing append streams, and flush.
//!
//! Dependency-free (`harness = false`, plain `main`) with the same counting allocator as the
//! heap bench — the allocs/op column shows that mapped I/O itself allocates nothing (the OS
//! pages back the mapping); only open/grow bookkeeping allocates.
//!
//! Run with `cargo bench -p yggdryl-core --bench io_local_mmap`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::local::Mmap;
use yggdryl_core::io::memory::IOBase;

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
    println!("  {name:<34} {mops:8.2}      {allocs:6.2}      {bytes:7.1}");
}

fn main() {
    let iters = 2_000; // file-backed: fewer iterations than the pure in-memory bench
    let path = std::env::temp_dir().join(format!("yggdryl_mmap_bench_{}.bin", std::process::id()));
    let page: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();

    println!("Mmap — time & memory ({iters} iters)\n");
    println!(
        "  {:<34} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // A persistent working mapping for the read/write rows.
    let mut map = Mmap::create_path(&path).expect("create bench mapping");
    map.pwrite_byte_array(0, &page);

    row(
        "pread_i32 (mapped)",
        measure(1, iters * 10, || {
            let _ = map.pread_i32(64).unwrap();
        }),
    );
    row(
        "pwrite_i32 (mapped, in place)",
        measure(1, iters * 10, || {
            map.pwrite_i32(64, -1).unwrap();
        }),
    );
    let mut scratch = Vec::with_capacity(page.len());
    row(
        "pread_into 4 KiB (mapped)",
        measure(1, iters, || {
            let _ = map.pread_into(0, page.len(), &mut scratch);
        }),
    );
    row(
        "overwrite 4 KiB (mapped)",
        measure(1, iters, || {
            let _ = map.pwrite_byte_array(0, &page);
        }),
    );
    let values = vec![7i32; 1024];
    let mut back = vec![0i32; 1024];
    row(
        "pwrite_i32_array (1024, mapped)",
        measure(1024, iters, || {
            map.pwrite_i32_array(0, &values).unwrap();
        }),
    );
    row(
        "pread_i32_array (1024, mapped)",
        measure(1024, iters, || {
            map.pread_i32_array(0, &mut back).unwrap();
        }),
    );
    drop(map);

    // Auto-resizing append stream: a fresh file grown 64 KiB in 1 KiB chunks per iteration —
    // the allocs/op column shows only bookkeeping, and growth remaps O(log n) times.
    row(
        "append 64x1 KiB (fresh file)",
        measure(64, iters / 10, || {
            let mut m = Mmap::create_path(&path).expect("create");
            let chunk = [0u8; 1024];
            for _ in 0..64 {
                let end = m.byte_size();
                let _ = m.pwrite_byte_array(end, &chunk);
            }
            drop(m);
            std::fs::remove_file(&path).ok();
        }),
    );

    // Open + close an existing 4 KiB file.
    {
        let mut m = Mmap::create_path(&path).expect("create");
        m.pwrite_byte_array(0, &page);
    }
    row(
        "open + close (4 KiB file)",
        measure(1, iters / 10, || {
            let _ = Mmap::open_path(&path).expect("open");
        }),
    );

    // Flush a dirty page to disk.
    let mut map = Mmap::open_path(&path).expect("open");
    row(
        "flush (4 KiB dirty)",
        measure(1, iters / 10, || {
            map.pwrite_i32(0, -1).unwrap();
            map.flush().unwrap();
        }),
    );
    drop(map);
    std::fs::remove_file(&path).ok();
}
