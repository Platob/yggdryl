//! Digesting a stored object, and what never holding it is worth.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::DigestAlgorithm;
use yggdryl::IOBase;
use yggdryl::io::Buffer;
use yggdryl::xxhash::{self, Hashed};

use super::payload;

/// The file every handle case reads, large enough that holding it shows.
const FILE_BYTES: usize = 64 * 1024 * 1024;

/// A temporary root for the local file the handle cases read.
fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("yggdryl-xxhash-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a writable temporary directory");
    path
}

/// The process's peak resident set, in bytes, when the kernel reports one.
fn peak_resident() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kilobytes * 1024)
}

/// Report what each way of digesting a stored object costs in memory.
///
/// The streamed read runs first and the whole read second, so the high-water
/// mark's increase is attributable to the second: the kernel's mark never
/// falls, which is exactly why the order matters. The gap is the reason
/// `read_digest` exists.
fn report_memory(path: &std::path::Path) {
    let handle = yggdryl::local::File::new(path).expect("a readable local file");
    let before = peak_resident();
    let streamed = handle
        .read_digest(DigestAlgorithm::Xxh3_64)
        .expect("the file digests");
    let after_streamed = peak_resident();
    let whole = handle.read_all_bytes().expect("the file reads");
    let digest = DigestAlgorithm::Xxh3_64.digest(&whole);
    let after_whole = peak_resident();
    assert_eq!(streamed, digest, "both ways answer one value");
    drop(whole);

    match (before, after_streamed, after_whole) {
        (Some(before), Some(streamed), Some(whole)) => {
            println!(
                "read_digest peak resident: {:.1} MiB before, {:.1} MiB after streaming, \
                 {:.1} MiB after read_all_bytes over a {} file",
                before as f64 / (1024.0 * 1024.0),
                streamed as f64 / (1024.0 * 1024.0),
                whole as f64 / (1024.0 * 1024.0),
                super::label(FILE_BYTES),
            );
        }
        _ => println!("peak resident bytes are unavailable on this platform"),
    }
}

/// Digesting a handle, against reading it whole and hashing the bytes.
pub(crate) fn handle_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let path = root.join("trades.jsonl");
    std::fs::write(&path, payload(FILE_BYTES)).expect("the fixture writes");
    report_memory(&path);

    let mut group = criterion.benchmark_group("xxhash_handle");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(FILE_BYTES as u64));
    group.bench_function("read_digest", |bencher| {
        let handle = yggdryl::local::File::new(&path).expect("a readable local file");
        bencher.iter(|| {
            black_box(&handle)
                .read_digest(DigestAlgorithm::Xxh3_64)
                .expect("the file digests")
        });
    });
    group.bench_function("read_all_bytes_then_digest", |bencher| {
        let handle = yggdryl::local::File::new(&path).expect("a readable local file");
        bencher.iter(|| {
            let bytes = black_box(&handle).read_all_bytes().expect("the file reads");
            DigestAlgorithm::Xxh3_64.digest(&bytes)
        });
    });
    group.finish();

    // `Hashed<H>` write-through against an unwrapped write plus a second pass.
    // The wrapper's whole claim is that the second pass disappears.
    let written = payload(4 * 1024 * 1024);
    let mut group = criterion.benchmark_group("xxhash_hashed");
    group.sample_size(20);
    group.throughput(Throughput::Bytes(written.len() as u64));
    group.bench_function("hashed_write_through", |bencher| {
        bencher.iter(|| {
            let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64);
            handle
                .write_all_bytes(black_box(&written))
                .expect("a memory buffer writes");
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .expect("the running state answers")
        });
    });
    group.bench_function("write_then_digest_pass", |bencher| {
        bencher.iter(|| {
            let mut handle = Buffer::new();
            handle
                .write_all_bytes(black_box(&written))
                .expect("a memory buffer writes");
            handle
                .read_digest(DigestAlgorithm::Xxh3_64)
                .expect("the buffer digests")
        });
    });
    group.finish();

    // The pass-through reader and writer, against moving the bytes alone.
    let mut group = criterion.benchmark_group("xxhash_stream_wrappers");
    group.throughput(Throughput::Bytes(written.len() as u64));
    group.bench_function("plain_copy", |bencher| {
        bencher.iter(|| {
            let mut target = Vec::with_capacity(written.len());
            std::io::copy(&mut black_box(written.as_slice()), &mut target)
                .expect("an in-memory copy")
        });
    });
    group.bench_function("digest_reader", |bencher| {
        bencher.iter(|| {
            let mut source = xxhash::reader(written.as_slice(), DigestAlgorithm::Xxh3_64);
            let mut target = Vec::with_capacity(written.len());
            std::io::copy(&mut source, &mut target).expect("an in-memory copy");
            source.as_digest()
        });
    });
    group.bench_function("digest_writer", |bencher| {
        bencher.iter(|| {
            let mut target =
                xxhash::writer(Vec::with_capacity(written.len()), DigestAlgorithm::Xxh3_64);
            std::io::copy(&mut black_box(written.as_slice()), &mut target)
                .expect("an in-memory copy");
            target.as_digest()
        });
    });
    group.finish();

    let _ = std::fs::remove_dir_all(&root);
}
