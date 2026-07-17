//! Time **and** memory benchmark for [`LocalIO`](yggdryl_core::io::local::LocalIO) — the
//! local-filesystem access point — and the concurrency story of the raw
//! [`Mmap`](yggdryl_core::io::local::Mmap) it self-optimizes onto.
//!
//! It measures what a real caller touches: the **lazy auto-create** first write (parents +
//! file + mapping brought into being on demand), the **self-optimized** mapped fast path that
//! follows, the gap between an **ad-hoc** read on a never-written handle and a **mapped** read,
//! the **SIMD bulk** typed arrays / repeats (raw `Mmap`'s direct contiguous conversion vs the
//! same op driven through `LocalIO`), the **memory-tree** directory reads, and **concurrency**
//! (many threads reading one shared mapping; many threads writing disjoint files).
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! heap/mmap benches, so single-threaded rows report **allocations/op** and **bytes/op** next
//! to throughput; concurrency rows report throughput only (a shared process-wide allocator
//! counter is not per-op meaningful across threads).
//!
//! Run with `cargo bench -p yggdryl-core --bench io_local_io` (release for real throughput).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Instant;

use yggdryl_core::io::local::{LocalIO, Mmap};
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

/// `(Mops/s, allocs/op, bytes/op)` over `iters` runs of `items` inputs each, warm-up excluded.
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
    // 4 significant-ish decimals so filesystem-bound rows (create, tree walk) still show a
    // number instead of rounding to 0.00.
    println!("  {name:<38} {mops:10.4}    {allocs:6.2}      {bytes:8.1}");
}

/// Throughput-only timing, for concurrency rows where a process-wide allocator counter is not
/// per-op meaningful. `items` is the total work across all threads per iteration.
fn time_only(items: usize, iters: u32, mut op: impl FnMut()) -> f64 {
    op();
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    items as f64 * f64::from(iters) / start.elapsed().as_secs_f64() / 1_000_000.0
}

fn trow(name: &str, mops: f64) {
    println!("  {name:<38} {mops:8.2}");
}

fn main() {
    let dir = std::env::temp_dir().join(format!("yggdryl_localio_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("bench dir");
    let iters = 2_000u32;
    let page: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();

    println!("LocalIO — time & memory ({iters} iters)\n");
    println!(
        "  {:<38} {:>8}   {:>10}   {:>10}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(78));

    // --- Lazy auto-create: the first write brings parents + file + mapping into being. Each
    // iteration targets a fresh nested path (a unique counter) so it is a true *first* write;
    // the whole subtree is removed once, at the end, outside the timed window. ---
    let lazy_n = std::cell::Cell::new(0u32);
    row(
        "lazy first write (mkdir -p + create + map)",
        measure(1, iters / 4, || {
            let n = lazy_n.get();
            lazy_n.set(n + 1);
            let mut node = LocalIO::from_path(dir.join(format!("lazy/d{n}/deep/note.bin")));
            node.pwrite_i64(0, 1 << 40).unwrap();
            node.close();
        }),
    );
    std::fs::remove_dir_all(dir.join("lazy")).ok();

    // A persistent self-optimized (mapped) handle for the fast-path rows.
    let mut hot = LocalIO::from_path(dir.join("hot.bin"));
    hot.pwrite_byte_array(0, &page);
    assert!(hot.is_mapped());

    row(
        "pread_i32 (mapped)",
        measure(1, iters * 10, || {
            let _ = hot.pread_i32(64).unwrap();
        }),
    );
    row(
        "pwrite_i32 (mapped, in place)",
        measure(1, iters * 10, || {
            hot.pwrite_i32(64, -1).unwrap();
        }),
    );
    row(
        "overwrite 4 KiB (mapped)",
        measure(1, iters, || {
            let _ = hot.pwrite_byte_array(0, &page);
        }),
    );

    // --- Ad-hoc vs mapped read: the self-optimization payoff. Same 4 KiB file, two handles. ---
    row(
        "pread 4 KiB (ad-hoc, never written)",
        measure(1, iters, || {
            let reader = LocalIO::from_path(dir.join("hot.bin"));
            let _ = reader.pread_vec(0, 4096);
        }),
    );
    let mut scratch = Vec::with_capacity(page.len());
    row(
        "pread_into 4 KiB (mapped, reused buf)",
        measure(1, iters, || {
            let _ = hot.pread_into(0, page.len(), &mut scratch);
        }),
    );

    // --- SIMD bulk: raw Mmap converts directly off the mapping; LocalIO drives the same op. ---
    let values = vec![7i32; 1024];
    let mut back = vec![0i32; 1024];
    let mut raw = Mmap::create_path(dir.join("raw.bin")).unwrap();
    raw.pwrite_i32_array(0, &values).unwrap();
    row(
        "Mmap pwrite_i32_array (1024, direct)",
        measure(1024, iters, || {
            raw.pwrite_i32_array(0, &values).unwrap();
        }),
    );
    row(
        "Mmap pread_i32_array (1024, direct)",
        measure(1024, iters, || {
            raw.pread_i32_array(0, &mut back).unwrap();
        }),
    );
    row(
        "LocalIO pwrite_i32_array (1024, mapped)",
        measure(1024, iters, || {
            hot.pwrite_i32_array(0, &values).unwrap();
        }),
    );
    row(
        "LocalIO pread_i32_array (1024, mapped)",
        measure(1024, iters, || {
            hot.pread_i32_array(0, &mut back).unwrap();
        }),
    );
    row(
        "Mmap pwrite_byte_repeat (8 KiB memset)",
        measure(8192, iters, || {
            raw.pwrite_byte_repeat(0, 0xAB, 8192).unwrap();
        }),
    );
    let values64 = vec![-1i64; 1024];
    row(
        "Mmap pwrite_i64_array (1024, direct)",
        measure(1024, iters, || {
            raw.pwrite_i64_array(0, &values64).unwrap();
        }),
    );

    // --- Memory tree: a directory of N file blocks read as one contiguous region. ---
    let tree = LocalIO::from_path(dir.join("tree"));
    for i in 0..16u32 {
        let mut block = tree.join_str(&format!("b{i:02}.bin"));
        block.pwrite_byte_array(0, &[i as u8; 256]);
        block.close();
    }
    row(
        "join (lazy child, Uri::joinpath)",
        measure(1, iters, || {
            let _ = tree.join("logs/2026/day.bin").unwrap();
        }),
    );
    row(
        "tree byte_size (16 blocks, lazy sum)",
        measure(1, iters, || {
            let _ = tree.byte_size();
        }),
    );
    let mut tree_buf = vec![0u8; 16 * 256];
    row(
        "tree pread whole (16x256, stitched)",
        measure(1, iters, || {
            let _ = tree.pread_byte_array(0, &mut tree_buf);
        }),
    );

    // --- Concurrency. ---
    println!("\nConcurrency — throughput (Mops/s)\n");
    println!("  {:<38} {:>8}", "op", "Mops/s");
    println!("  {}", "-".repeat(48));

    // Many threads reading ONE shared mapping (`&self` reads are `Sync`, zero contention).
    let shared = Arc::new(Mmap::open_path(dir.join("hot.bin")).unwrap());
    for threads in [1usize, 2, 4, 8] {
        let reads_per_thread = 1_000_000usize;
        let mops = time_only(threads * reads_per_thread, 20, || {
            std::thread::scope(|s| {
                for _ in 0..threads {
                    let m = Arc::clone(&shared);
                    s.spawn(move || {
                        let mut acc = 0i64;
                        for k in 0..reads_per_thread {
                            acc ^= m.pread_i32(((k % 1000) * 4) as u64).unwrap() as i64;
                        }
                        std::hint::black_box(acc);
                    });
                }
            });
        });
        trow(&format!("shared-mapping reads x{threads} threads"), mops);
    }

    // Many threads each writing its OWN disjoint file through its own LocalIO handle.
    for threads in [1usize, 2, 4, 8] {
        let writes_per_thread = 500_000usize;
        let mops = time_only(threads * writes_per_thread, 20, || {
            std::thread::scope(|s| {
                for t in 0..threads {
                    let path = dir.join(format!("conc_{t}.bin"));
                    s.spawn(move || {
                        let mut node = LocalIO::from_path(&path);
                        for k in 0..writes_per_thread {
                            node.pwrite_i32(((k % 1000) * 4) as u64, k as i32).unwrap();
                        }
                        node.close();
                    });
                }
            });
        });
        trow(&format!("disjoint-file writes x{threads} threads"), mops);
    }

    std::fs::remove_dir_all(&dir).ok();
}
