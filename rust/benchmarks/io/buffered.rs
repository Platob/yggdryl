//! What a page cache buys over a handle, and what it costs.
//!
//! Three workloads over one 16 MiB fixture, each run over three handles so the
//! comparison is like for like: an in-memory [`Buffer`] as the floor a cache
//! can never beat, a bare memory-mapped [`File`](yggdryl::local::File), and
//! that same file wrapped in [`Buffered`]. Every case reports the bytes it
//! moves as its Criterion throughput, so the rows are comparable across
//! workloads and not only within one.
//!
//! - `random` reads 512 bytes at a time from a 4 MiB hot region, which fits
//!   inside the 8 MiB default budget. After the first iteration every page is
//!   held, so this is the cache's own hit path against the two uncached ones.
//! - `sequential` scans the whole 16 MiB in 8 KiB steps. Twice the budget, so
//!   every page is fetched exactly once and evicted before it is asked for
//!   again: a pure miss workload, and therefore the price of the cache with
//!   none of its benefit.
//! - `footer` is the shape a footer-first container is opened with - read the
//!   tail, read the head, scan a chunk of the middle, then read both ends
//!   again - which is what the pinned header and footer pages are for. The
//!   middle scan is sized to sweep the budget, so an unpinned cache would have
//!   dropped both ends by the time they are asked for the second time.
//!
//! Read the numbers for what they are. A `pread` on a mapped file is already
//! a `memcpy` out of the page cache the *kernel* keeps, so a user-space cache
//! over one is a second copy plus a lock; what these rows measure is that
//! overhead against the per-read work it removes, on the one backend the core
//! ships. A handle whose fetch is a network round trip is the case the cache
//! exists for, and it is not this table - nothing here should be read as a
//! claim about one.

use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{Criterion, Throughput};
use yggdryl::buffered::{Buffered, BufferedOptions};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::local::File;

/// The fixture's size: twice the default byte budget, so a full scan evicts.
const FIXTURE: u64 = 16 * 1024 * 1024;
/// The region the random reads stay inside, which fits the default budget.
const HOT: u64 = 4 * 1024 * 1024;
/// One random read.
const SMALL: usize = 512;
/// One sequential read.
const STEP: usize = 8 * 1024;
/// Reads per random-access iteration.
const DRAWS: usize = 1_024;
/// Bytes the middle scan of the footer workload sweeps, past the budget.
const SWEEP: u64 = 12 * 1024 * 1024;
/// Bytes each end of the footer workload reads.
const ENDS: usize = 8 * 1024;

/// A handle that mirrors a [`Buffer`] and counts the reads reaching it.
///
/// What pinning buys is a *fetch that does not happen*, and no timing over an
/// in-core backend can show that: a mapped re-read is a `memcpy` either way.
/// So the claim is carried by a count, asserted before any timer starts, the
/// way the pushdown target asserts its materialized bytes.
struct Counting {
    handle: Buffer,
    reads: AtomicUsize,
}

impl IOBase for Counting {
    yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate,
        url, media_type, set_media_type, flush, parent, child_by, ls, kind);

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.handle.pread(offset, buffer)
    }
}

/// Prove the pinned ends survive the workload the `footer` cases time.
///
/// One pass warms both ends and sweeps the middle; the second repeats it. If
/// the pins held, the second pass's reads of the head and the tail cost no
/// inner read at all, and the difference between the two passes is exactly
/// the middle scan.
fn pinning_holds_both_ends(payload: &[u8]) -> (usize, usize) {
    let cached = Counting {
        handle: Buffer::from_bytes(payload.to_vec()),
        reads: AtomicUsize::new(0),
    }
    .buffered(BufferedOptions::default());

    let mut ends = vec![0_u8; ENDS];
    let sweep = |handle: &dyn IOBase| {
        let mut middle = vec![0_u8; STEP];
        let mut offset = ENDS as u64;
        while offset < SWEEP + ENDS as u64 {
            handle
                .pread(offset, &mut middle)
                .expect("a readable fixture");
            offset += STEP as u64;
        }
    };

    cached
        .pread(FIXTURE - ENDS as u64, &mut ends)
        .expect("a readable fixture");
    cached.pread(0, &mut ends).expect("a readable fixture");
    sweep(&cached);
    let warmed = cached.handle().reads.load(Ordering::Relaxed);

    // The two ends again, after a scan four times the budget wide.
    cached
        .pread(FIXTURE - ENDS as u64, &mut ends)
        .expect("a readable fixture");
    cached.pread(0, &mut ends).expect("a readable fixture");
    let after_ends = cached.handle().reads.load(Ordering::Relaxed);

    sweep(&cached);
    let after_sweep = cached.handle().reads.load(Ordering::Relaxed);
    assert_eq!(
        warmed, after_ends,
        "the pinned head and footer pages must cost no repeat fetch"
    );
    assert!(
        after_sweep > after_ends,
        "the middle must genuinely have been evicted"
    );
    (after_ends - warmed, after_sweep - after_ends)
}

pub(crate) fn buffered_benchmarks(criterion: &mut Criterion) {
    let path =
        std::env::temp_dir().join(format!("yggdryl-bench-buffered-{}.bin", std::process::id()));
    let payload = fixture();
    std::fs::write(&path, &payload).expect("the fixture must be written");

    // The claim the `footer` cases cannot time, established as a count first:
    // the two ends cost nothing to re-read, while the middle - a scan wider
    // than the budget, so every page of it has been evicted by the time it
    // comes round again - is fetched afresh, at least a budget's worth of it.
    let budget = BufferedOptions::default();
    let held_pages = budget.max_bytes() / budget.page_size() as u64;
    let (ends_refetched, middle_refetched) = pinning_holds_both_ends(&payload);
    assert_eq!(ends_refetched, 0, "a pinned end must never be re-fetched");
    assert!(
        middle_refetched as u64 >= held_pages,
        "the middle scan must genuinely have swept the budget, \
         got {middle_refetched} fetches against {held_pages} pages of budget"
    );

    let memory = Buffer::from_bytes(payload);
    let file = File::new(&path).expect("a local fixture path");
    let cached = File::new(&path)
        .expect("a local fixture path")
        .buffered(BufferedOptions::default());
    let offsets = draws();

    let mut group = criterion.benchmark_group("io_buffered");

    group.throughput(Throughput::Bytes((DRAWS * SMALL) as u64));
    for (label, handle) in cases(&memory, &file, &cached) {
        group.bench_function(format!("random/{label}"), |bencher| {
            bencher.iter(|| random(black_box(handle), &offsets));
        });
    }

    group.throughput(Throughput::Bytes(FIXTURE));
    for (label, handle) in cases(&memory, &file, &cached) {
        group.bench_function(format!("sequential/{label}"), |bencher| {
            bencher.iter(|| sequential(black_box(handle)));
        });
    }

    // The pinning case: the two ends against a middle that sweeps the budget.
    group.throughput(Throughput::Bytes(SWEEP + 4 * ENDS as u64));
    for (label, handle) in cases(&memory, &file, &cached) {
        group.bench_function(format!("footer/{label}"), |bencher| {
            bencher.iter(|| footer(black_box(handle)));
        });
    }

    group.finish();

    drop(cached);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

/// The three handles every workload runs over, in one order.
fn cases<'handles>(
    memory: &'handles Buffer,
    file: &'handles File,
    cached: &'handles Buffered<File>,
) -> [(&'static str, &'handles dyn IOBase); 3] {
    [("buffer", memory), ("file", file), ("buffered", cached)]
}

/// The fixture's bytes, generated so the value is not one repeated page.
fn fixture() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FIXTURE as usize);
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    while bytes.len() < FIXTURE as usize {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(FIXTURE as usize);
    bytes
}

/// The offsets the random workload reads, drawn once outside every timer.
fn draws() -> Vec<u64> {
    let mut offsets = Vec::with_capacity(DRAWS);
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..DRAWS {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        offsets.push((state >> 16) % (HOT - SMALL as u64));
    }
    offsets
}

/// Read `SMALL` bytes at each drawn offset.
fn random(handle: &dyn IOBase, offsets: &[u64]) -> usize {
    let mut target = [0_u8; SMALL];
    let mut read = 0;
    for offset in offsets {
        read += handle
            .pread(*offset, &mut target)
            .expect("a readable fixture");
    }
    read
}

/// Read the whole fixture in `STEP`-sized steps.
fn sequential(handle: &dyn IOBase) -> usize {
    let mut target = vec![0_u8; STEP];
    let mut offset = 0;
    let mut read = 0;
    while offset < FIXTURE {
        let count = handle
            .pread(offset, &mut target)
            .expect("a readable fixture");
        if count == 0 {
            break;
        }
        offset += count as u64;
        read += count;
    }
    read
}

/// Read both ends, sweep the middle past the budget, read both ends again.
fn footer(handle: &dyn IOBase) -> usize {
    let mut ends = vec![0_u8; ENDS];
    let mut middle = vec![0_u8; STEP];
    let mut read = 0;

    for _ in 0..2 {
        // The tail first, then the head: a Parquet open reads its footer
        // before it knows what the head holds.
        read += handle
            .pread(FIXTURE - ENDS as u64, &mut ends)
            .expect("a readable fixture");
        read += handle.pread(0, &mut ends).expect("a readable fixture");

        let mut offset = ENDS as u64;
        while offset < SWEEP / 2 + ENDS as u64 {
            read += handle
                .pread(offset, &mut middle)
                .expect("a readable fixture");
            offset += STEP as u64;
        }
    }
    read
}
