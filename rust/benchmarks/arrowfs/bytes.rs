//! Byte-level wrapper overhead, against the handle each backend wraps.
//!
//! The memory legs compare an `arrowfs` handle over [`MemoryFileSystem`]
//! against the native [`Buffer`](yggdryl::io::Buffer) - both hold their bytes
//! in memory, so the difference is purely the vtable plus the staging. The
//! local legs compare an `arrowfs` handle over [`LocalFileSystem`] against
//! [`local::File`](yggdryl::local::File), the memory-mapped local backend, on
//! the same payload.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::arrowfs::File as ArrowFile;
use yggdryl::io::IOBase;

use super::{PAYLOAD, buffer, local, local_location, memory, payload};

pub(crate) fn byte_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("arrowfs_bytes");
    group.throughput(Throughput::Bytes(PAYLOAD as u64));

    let bytes = payload();

    // A whole-value read: one ranged fetch through the vtable, against the
    // slice copy the native buffer does.
    group.bench_function("read_all/arrowfs_memory", |bencher| {
        let filesystem = memory();
        let mut handle =
            ArrowFile::from_location(filesystem, "bench/read.bin").expect("a valid location");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.close().expect("the fixture publishes");
        bencher.iter(|| {
            black_box(&handle)
                .read_all()
                .expect("a readable value")
                .len()
        });
    });

    group.bench_function("read_all/buffer", |bencher| {
        let mut handle = buffer("read.bin");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        bencher.iter(|| {
            black_box(&handle)
                .read_all()
                .expect("a readable value")
                .len()
        });
    });

    // A ranged read, which is the shape a footer-first reader uses: the
    // wrapper must not download the whole value to serve one range.
    group.bench_function("pread_range/arrowfs_memory", |bencher| {
        let filesystem = memory();
        let mut handle =
            ArrowFile::from_location(filesystem, "bench/range.bin").expect("a valid location");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.close().expect("the fixture publishes");
        let mut target = vec![0_u8; 4096];
        bencher.iter(|| {
            black_box(&handle)
                .pread(black_box(PAYLOAD as u64 - 4096), &mut target)
                .expect("a readable range")
        });
    });

    group.bench_function("pread_range/buffer", |bencher| {
        let mut handle = buffer("range.bin");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        let mut target = vec![0_u8; 4096];
        bencher.iter(|| {
            black_box(&handle)
                .pread(black_box(PAYLOAD as u64 - 4096), &mut target)
                .expect("a readable range")
        });
    });

    // A whole-value write. The wrapper stages and publishes exactly once on
    // close, so the close is inside the measured region.
    group.bench_function("write_all/arrowfs_memory", |bencher| {
        let filesystem = memory();
        bencher.iter(|| {
            let mut handle = ArrowFile::from_location(filesystem.clone(), "bench/write.bin")
                .expect("a valid location");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
            handle.close().expect("the write publishes");
        });
    });

    group.bench_function("write_all/buffer", |bencher| {
        bencher.iter(|| {
            let mut handle = buffer("write.bin");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
        });
    });

    let (filesystem, root) = local();

    group.bench_function("read_all/arrowfs_local", |bencher| {
        let location = local_location(&root, "read.bin");
        let mut handle =
            ArrowFile::from_location(filesystem.clone(), &location).expect("a valid location");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.close().expect("the fixture publishes");
        bencher.iter(|| {
            black_box(&handle)
                .read_all()
                .expect("a readable value")
                .len()
        });
    });

    group.bench_function("read_all/local_file", |bencher| {
        let path = root.join("read-mapped.bin");
        let mut handle = yggdryl::local::File::create(&path).expect("a valid path");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.flush().expect("the fixture publishes");
        bencher.iter(|| {
            black_box(&handle)
                .read_all()
                .expect("a readable value")
                .len()
        });
    });

    group.bench_function("write_all/arrowfs_local", |bencher| {
        let location = local_location(&root, "write.bin");
        bencher.iter(|| {
            let mut handle =
                ArrowFile::from_location(filesystem.clone(), &location).expect("a valid location");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
            handle.close().expect("the write publishes");
        });
    });

    group.bench_function("write_all/local_file", |bencher| {
        let path = root.join("write-mapped.bin");
        bencher.iter(|| {
            let mut handle = yggdryl::local::File::create(&path).expect("a valid path");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
            handle.flush().expect("the write publishes");
        });
    });

    group.finish();
    let _ = std::fs::remove_dir_all(&root);
}
