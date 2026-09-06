//! Byte-level wrapper overhead, against the handle each backend wraps.
//!
//! The memory legs compare an `fs` handle over [`MemoryFileSystem`]
//! against the native [`Buffer`](yggdryl::holder::Buffer) - both hold their bytes
//! in memory, so the difference is purely the vtable plus stream dispatch. The
//! local legs compare an `fs` handle over [`LocalFileSystem`] against
//! [`local::File`](yggdryl::holder::local::File), the memory-mapped local backend, on
//! the same payload.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::IOBase;
use yggdryl::holder::fs::File as FsFile;

use super::{PAYLOAD, buffer, local, local_location, memory, payload};

pub(crate) fn byte_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fs_bytes");
    group.throughput(Throughput::Bytes(PAYLOAD as u64));

    let bytes = payload();

    // A whole-value read: one sequential input stream through the vtable,
    // against the slice copy the native buffer does.
    group.bench_function("read_all/fs_memory", |bencher| {
        let filesystem = memory();
        let mut handle =
            FsFile::from_path(filesystem, "bench/read.bin", None).expect("a valid location");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.close().expect("the fixture publishes");
        // The whole value is black-boxed rather than its length: observing
        // only the length lets the optimizer delete the allocation and the
        // copy outright, which would time nothing and read as a win.
        bencher.iter(|| {
            black_box(
                black_box(&handle)
                    .read_all_bytes()
                    .expect("a readable value"),
            )
        });
    });

    group.bench_function("read_all/buffer", |bencher| {
        let mut handle = buffer("read.bin");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        // The whole value is black-boxed rather than its length: observing
        // only the length lets the optimizer delete the allocation and the
        // copy outright, which would time nothing and read as a win.
        bencher.iter(|| {
            black_box(
                black_box(&handle)
                    .read_all_bytes()
                    .expect("a readable value"),
            )
        });
    });

    // A ranged read, which is the shape a footer-first reader uses: the
    // wrapper must not download the whole value to serve one range.
    group.bench_function("pread_range/fs_memory", |bencher| {
        let filesystem = memory();
        let mut handle =
            FsFile::from_path(filesystem, "bench/range.bin", None).expect("a valid location");
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

    // A whole-value write forwarded through one output stream. Stream close
    // is part of the filesystem operation and remains inside the measurement.
    group.bench_function("write_all/fs_memory", |bencher| {
        let filesystem = memory();
        bencher.iter(|| {
            let mut handle = FsFile::from_path(filesystem.clone(), "bench/write.bin", None)
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

    group.bench_function("read_all/fs_local", |bencher| {
        let location = local_location(&root, "read.bin");
        let mut handle =
            FsFile::from_path(filesystem.clone(), &location, None).expect("a valid location");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.close().expect("the fixture publishes");
        // The whole value is black-boxed rather than its length: observing
        // only the length lets the optimizer delete the allocation and the
        // copy outright, which would time nothing and read as a win.
        bencher.iter(|| {
            black_box(
                black_box(&handle)
                    .read_all_bytes()
                    .expect("a readable value"),
            )
        });
    });

    group.bench_function("read_all/local_file", |bencher| {
        let path = root.join("read-mapped.bin");
        let mut handle = yggdryl::holder::local::File::create(&path).expect("a valid path");
        handle.write_all_bytes(&bytes).expect("the fixture writes");
        handle.flush().expect("the fixture publishes");
        // The whole value is black-boxed rather than its length: observing
        // only the length lets the optimizer delete the allocation and the
        // copy outright, which would time nothing and read as a win.
        bencher.iter(|| {
            black_box(
                black_box(&handle)
                    .read_all_bytes()
                    .expect("a readable value"),
            )
        });
    });

    group.bench_function("write_all/fs_local", |bencher| {
        let location = local_location(&root, "write.bin");
        bencher.iter(|| {
            let mut handle =
                FsFile::from_path(filesystem.clone(), &location, None).expect("a valid location");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
            handle.close().expect("the write publishes");
        });
    });

    group.bench_function("write_all/local_file", |bencher| {
        let path = root.join("write-mapped.bin");
        bencher.iter(|| {
            let mut handle = yggdryl::holder::local::File::create(&path).expect("a valid path");
            handle
                .write_all_bytes(black_box(&bytes))
                .expect("a writable value");
            handle.flush().expect("the write publishes");
        });
    });

    group.finish();
    let _ = std::fs::remove_dir_all(&root);
}
