//! Benchmarks for the cursor and slice adapters, measured against direct positioned
//! access on the underlying `ByteBuffer` so their overhead is visible.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_core::{ByteBuffer, RawIOBase, Seekable, Whence};

const N: usize = 4096;
const CHUNK: usize = 64;

fn cursor(c: &mut Criterion) {
    let mut group = c.benchmark_group("cursor");
    group.throughput(Throughput::Bytes(N as u64));

    // Baseline: the same sequential chunked scan with explicit positions.
    let buf = ByteBuffer::from_bytes(vec![0xABu8; N]);
    group.bench_function("direct_chunked_scan", |b| {
        b.iter(|| {
            for chunk in 0..N / CHUNK {
                black_box(
                    buf.pread_byte_array(chunk * CHUNK, Whence::Start, CHUNK)
                        .unwrap(),
                );
            }
        })
    });

    // The cursor tracks the position itself: read at `Current`, advance, repeat.
    let mut cursor = ByteBuffer::from_bytes(vec![0xABu8; N]).cursor();
    group.bench_function("cursor_chunked_scan", |b| {
        b.iter(|| {
            cursor.seek(0, Whence::Start).unwrap();
            for _ in 0..N / CHUNK {
                black_box(cursor.pread_byte_array(0, Whence::Current, CHUNK).unwrap());
            }
        })
    });

    group.finish();
}

fn slice(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice");
    group.throughput(Throughput::Bytes(N as u64));

    // A window into the middle of a larger buffer.
    let buf = ByteBuffer::from_bytes(vec![0x5Au8; 2 * N]);
    group.bench_function("direct_window_read", |b| {
        b.iter(|| {
            black_box(buf.pread_byte_array(N / 2, Whence::Start, N).unwrap());
        })
    });

    let window = ByteBuffer::from_bytes(vec![0x5Au8; 2 * N]).slice(N / 2, N / 2 + N);
    group.bench_function("slice_window_read", |b| {
        b.iter(|| {
            black_box(window.pread_byte_array(0, Whence::Start, N).unwrap());
        })
    });

    group.finish();
}

criterion_group!(benches, cursor, slice);
criterion_main!(benches);
