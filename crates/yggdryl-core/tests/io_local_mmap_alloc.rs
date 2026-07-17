//! Deterministic allocation budgets for the mapped local sources
//! ([`Mmap`](yggdryl_core::io::local::Mmap) and the mapped phase of
//! [`LocalIO`](yggdryl_core::io::local::LocalIO)) — the executable proof behind the docs'
//! "mapped I/O allocates nothing": once the mapping is live, every positioned, typed, bulk,
//! repeat, and cursor operation runs at zero heap allocations (the OS pages back the
//! mapping; growth bookkeeping happens outside the hot loop).
//!
//! This file is its own test binary with its own counting global allocator, and holds a
//! **single** `#[test]` so nothing else allocates on another thread while a region is
//! measured. (Throughput lives in the `io_local_mmap` bench.)

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use yggdryl_core::io::local::{LocalIO, Mmap};
use yggdryl_core::io::memory::IOBase;

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

/// Total allocations `op` makes over `iters` runs, after one warm-up run so any one-time
/// initialization stays outside the measured window.
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
    let dir = std::env::temp_dir().join(format!("yggdryl_mmap_alloc_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    {
        let mut map = Mmap::create_path(dir.join("map.bin")).unwrap();
        map.try_reserve(16 * 1024).unwrap(); // size once, outside every measured window
        map.pwrite_byte_array(0, &[0u8; 64]);

        // Typed positioned round-trips over a live mapping — zero allocation.
        let typed = allocs_over(iters, || {
            map.pwrite_byte(0, 1).unwrap();
            map.pwrite_i32(1, -7).unwrap();
            map.pwrite_i64(5, 1 << 40).unwrap();
            let _ = map.pread_byte(0).unwrap();
            let _ = map.pread_i32(1).unwrap();
            let _ = map.pread_i64(5).unwrap();
            let _ = map.pread_bit(3).unwrap();
        });
        assert_eq!(
            typed, 0,
            "typed mapped I/O must be zero-alloc (got {typed})"
        );

        // Bulk typed arrays stage through fixed STACK chunks — zero heap allocation.
        let bulk_values = vec![7i32; 1000];
        let mut bulk_back = vec![0i32; 1000];
        let bulk = allocs_over(iters, || {
            map.pwrite_i32_array(0, &bulk_values).unwrap();
            map.pread_i32_array(0, &mut bulk_back).unwrap();
        });
        assert_eq!(
            bulk, 0,
            "bulk mapped arrays must be zero-alloc (got {bulk})"
        );

        // Repeated-value fills never materialize the full array.
        let fills = allocs_over(iters, || {
            map.pwrite_byte_repeat(0, 0xAB, 8000).unwrap();
            map.pwrite_i64_repeat(0, -1, 1000).unwrap();
        });
        assert_eq!(
            fills, 0,
            "mapped repeat fills must be zero-alloc (got {fills})"
        );

        // pread_into reuses the caller's buffer across an entire transfer loop.
        let mut scratch = Vec::with_capacity(64);
        let transfer = allocs_over(iters, || {
            let _ = map.pread_into(0, 32, &mut scratch);
        });
        assert_eq!(transfer, 0, "mapped pread_into must reuse (got {transfer})");

        // The cursor stream over the mapping allocates nothing either.
        let cursor = allocs_over(iters, || {
            map.rewind();
            map.write_i64(1 << 33).unwrap();
            map.rewind();
            let _ = map.read_i64().unwrap();
        });
        assert_eq!(
            cursor, 0,
            "mapped cursor round-trip must be zero-alloc (got {cursor})"
        );
    }

    // LocalIO in its mapped (self-optimized) phase inherits the same budgets through its
    // delegation — the per-call `match` on the kept mapping adds no allocation.
    {
        let mut node = LocalIO::from_path(dir.join("node.bin"));
        node.try_reserve(16 * 1024).unwrap(); // first write: create + map, outside the window
        node.pwrite_byte_array(0, &[0u8; 64]);
        assert!(node.is_mapped());

        let mapped_node = allocs_over(iters, || {
            node.pwrite_i64(0, -1).unwrap();
            let _ = node.pread_i64(0).unwrap();
            node.pwrite_byte_repeat(8, 0xCD, 4096).unwrap();
        });
        assert_eq!(
            mapped_node, 0,
            "mapped LocalIO I/O must be zero-alloc (got {mapped_node})"
        );
        node.close();
    }

    std::fs::remove_dir_all(&dir).ok();
}
