//! What a page cache buys over a handle, and what it costs.
//!
//! Three workloads over one 16 MiB fixture, each run over every handle the
//! core ships, so the comparison is like for like. Every case reports the
//! bytes it moves as its Criterion throughput, so rows are comparable across
//! workloads and not only within one.
//!
//! The handles fall into two families, and that is the whole argument:
//!
//! - **Already memory.** An in-memory [`Buffer`] is the floor a cache can
//!   never beat, and a memory-mapped [`File`](yggdryl::holder::local::File) is a
//!   `memcpy` out of the page cache the *kernel* already keeps. Wrapping
//!   either can only add a lock, a clock read, a map lookup, and a second
//!   copy. These rows measure that overhead honestly.
//! - **A fetch per read.** An [`FsFile`](yggdryl::holder::fs::File) answers
//!   every `pread` with one random-access input-file open and one positional
//!   read through the filesystem vtable. Over [`MemoryFileSystem`] that is a
//!   lock and a copy; over [`LocalFileSystem`] it is an `open`, a `seek`, and a
//!   `read` **per call**, which is the shape every real object store has - a
//!   round trip whose cost dwarfs the bytes it carries. These are the rows the
//!   cache exists for.
//!
//! The workloads:
//!
//! - `random` reads 512 bytes at a time from a 4 MiB hot region, which fits
//!   inside the 8 MiB default budget. After the first iteration every page is
//!   held, so this is the cache's own hit path against the uncached ones.
//! - `sequential` scans the whole 16 MiB in 8 KiB steps. Twice the budget, so
//!   every page is fetched exactly once and evicted before it is asked for
//!   again: a pure miss workload, and therefore the price of the cache with
//!   none of its benefit - except over a fetch-per-read backend, where it
//!   still turns eight reads into one.
//! - `footer` is the shape a footer-first container is opened with - read the
//!   tail, read the head, scan a chunk of the middle, then read both ends
//!   again - which is what the pinned header and footer pages are for. The
//!   middle scan is sized to sweep the budget, so an unpinned cache would have
//!   dropped both ends by the time they are asked for the second time.
//!
//! Nothing here is a claim about a *network* filesystem; the local vtable is
//! a syscall per read, not a round trip over a wire. It is the closest thing
//! the core ships, it is inspectable, and it is the right shape.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{Criterion, Throughput};
use yggdryl::IOBase;
use yggdryl::coding::gzip::Gzip;
use yggdryl::holder::Buffer;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::fs::{File as FsFile, FileSystem, LocalFileSystem, MemoryFileSystem};
use yggdryl::holder::local::{File, Folder};

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
/// Where the memory filesystem keeps the fixture.
const MEMORY_LOCATION: &str = "bench/buffered.bin";
/// The decoded size of the compressed fixture.
///
/// Far smaller than the main one, because the uncached coded case decodes the
/// whole value once per read: at 16 MiB a single iteration would take longer
/// than the whole rest of the target.
const CODED: usize = 256 * 1024;
/// Reads per iteration of the coded workload.
const CODED_READS: usize = 64;
/// One coded read.
const CODED_STEP: usize = 4 * 1024;

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

impl yggdryl::IOMedia for Counting {
    yggdryl::delegate_iomedia!(handle);
}

impl IOBase for Counting {
    yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate,
        url, media_type, set_media_type, flush, parent, child_by_path, ls, kind);

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
    let path = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-bench-buffered-{}.bin", std::process::id()));
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

    let handles = handles(&path, payload);
    let offsets = draws();

    let mut group = criterion.benchmark_group("io_buffered");

    group.throughput(Throughput::Bytes((DRAWS * SMALL) as u64));
    for (label, handle) in &handles {
        group.bench_function(format!("random/{label}"), |bencher| {
            bencher.iter(|| random(black_box(handle.as_ref()), &offsets));
        });
    }

    group.throughput(Throughput::Bytes(FIXTURE));
    for (label, handle) in &handles {
        group.bench_function(format!("sequential/{label}"), |bencher| {
            bencher.iter(|| sequential(black_box(handle.as_ref())));
        });
    }

    // The pinning case: the two ends against a middle that sweeps the budget.
    group.throughput(Throughput::Bytes(SWEEP + 4 * ENDS as u64));
    for (label, handle) in &handles {
        group.bench_function(format!("footer/{label}"), |bencher| {
            bencher.iter(|| footer(black_box(handle.as_ref())));
        });
    }

    coded_cases(&mut group);

    group.finish();

    drop(handles);
    let _ = std::fs::remove_file(&path);
}

/// What a page cache is worth over a *compressed* handle.
///
/// A content coding is not seekable, so [`Coding`](yggdryl::coding::Coding) answers
/// a positional read by decoding the value. Which decode you pay depends on
/// whether the handle is open, and that is the whole table:
///
/// - `closed` retains nothing: every `pread` starts a decoder and stops after
///   its requested range. Sixty-four progressive reads still mean sixty-four
///   decoder starts and repeatedly discarded prefixes.
/// - `open` is the cure the coding already ships: `open` materializes the
///   decoded value and `close` releases it, so each read is a range copy out
///   of it. This is the path a caller who knows they are reading a compressed
///   value should take.
/// - `buffered` is what the page cache buys when the handle is *not* opened -
///   the case a caller who does not know what they were handed is in. The
///   cache turns one decoder start per read into one per page miss, and here
///   the whole value is four pages, so after the first pass there are none.
///
/// The order of wrapping is the useful one: `Buffered<Coding<_>>` caches the
/// *decoded* bytes. The other way round would cache the compressed bytes and
/// still decode on every read.
fn coded_cases(group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>) {
    let payload = coded_payload();
    let encoded = encoded_fixture(&payload);

    group.throughput(Throughput::Bytes((CODED_READS * CODED_STEP) as u64));

    group.bench_function("coded/closed", |bencher| {
        let handle = Gzip::new(Buffer::from_bytes(encoded.clone()));
        assert_eq!(handle.size(), CODED as u64);
        bencher.iter(|| coded_scan(black_box(&handle)));
    });

    group.bench_function("coded/open", |bencher| {
        let mut handle = Gzip::new(Buffer::from_bytes(encoded.clone()));
        handle.open().expect("a decodable fixture");
        bencher.iter(|| coded_scan(black_box(&handle)));
    });

    group.bench_function("coded/buffered", |bencher| {
        let handle =
            Gzip::new(Buffer::from_bytes(encoded.clone())).buffered(BufferedOptions::default());
        bencher.iter(|| coded_scan(black_box(&handle)));
    });
}

/// A payload worth compressing: repetitive, like every log and every column.
fn coded_payload() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CODED);
    let mut row = 0_u64;
    while bytes.len() < CODED {
        bytes
            .extend_from_slice(format!("{row:08},AAPL,{:.2},XNAS\n", row as f64 * 0.01).as_bytes());
        row += 1;
    }
    bytes.truncate(CODED);
    bytes
}

/// The gzip bytes of the coded fixture, encoded once outside every timer.
fn encoded_fixture(payload: &[u8]) -> Vec<u8> {
    let mut handle = Gzip::new(Buffer::new());
    handle
        .write_all_bytes(payload)
        .expect("the fixture must write");
    handle.flush().expect("the fixture must publish");
    let stored = handle.into_handle().expect("a published fixture");
    assert!(
        stored.size() < CODED as u64,
        "the coded fixture must actually compress"
    );
    stored.into_bytes()
}

/// Read the coded value in `CODED_STEP` steps, front to back.
fn coded_scan(handle: &dyn IOBase) -> usize {
    let mut target = vec![0_u8; CODED_STEP];
    let mut read = 0;
    for step in 0..CODED_READS {
        read += handle
            .pread((step * CODED_STEP) as u64, &mut target)
            .expect("a readable fixture");
    }
    read
}

/// Every handle the workloads run over, in one order, each already holding
/// the fixture.
///
/// They are boxed because the set spans four types and one battery runs over
/// all of them; `IOBase` is implemented for the box, so the byte half of the
/// contract forwards unchanged. The two `fs` filesystems are what turn
/// this from an overhead table into a comparison: over `LocalFileSystem`
/// every `pread` is its own `open`/`seek`/`read`, so what the cache removes
/// is a syscall rather than a copy.
fn handles(path: &std::path::Path, payload: Vec<u8>) -> Vec<(&'static str, Box<dyn IOBase>)> {
    // One shared memory filesystem, written once through the vtable, so both
    // memory legs read the same object.
    let in_memory: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    in_memory
        .create_dir("bench", true)
        .expect("the memory fixture root must be created");
    crate::fs::write_file(in_memory.as_ref(), MEMORY_LOCATION, &payload);

    // The local filesystem reads the very file the mapped legs read, so the
    // two local rows differ in how they reach the bytes and in nothing else.
    let on_disk: Arc<dyn FileSystem> = Arc::new(LocalFileSystem::new());
    let location = path.to_string_lossy().replace('\\', "/");

    vec![
        (
            "buffer",
            Box::new(Buffer::from_bytes(payload)) as Box<dyn IOBase>,
        ),
        (
            "file",
            Box::new(File::new(path).expect("a local fixture path")),
        ),
        (
            "buffered",
            Box::new(
                File::new(path)
                    .expect("a local fixture path")
                    .buffered(BufferedOptions::default()),
            ),
        ),
        (
            "fs_memory",
            Box::new(
                FsFile::from_path(Arc::clone(&in_memory), MEMORY_LOCATION, None)
                    .expect("a valid location"),
            ),
        ),
        (
            "fs_memory_buffered",
            Box::new(
                FsFile::from_path(in_memory, MEMORY_LOCATION, None)
                    .expect("a valid location")
                    .buffered(BufferedOptions::default()),
            ),
        ),
        (
            "fs_local",
            Box::new(
                FsFile::from_path(Arc::clone(&on_disk), &location, None).expect("a valid location"),
            ),
        ),
        (
            "fs_local_buffered",
            Box::new(
                FsFile::from_path(on_disk, &location, None)
                    .expect("a valid location")
                    .buffered(BufferedOptions::default()),
            ),
        ),
    ]
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
