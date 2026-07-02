//! Benchmarks for the `ByteBuffer` and `BitBuffer` resources.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_core::{BitBuffer, ByteBuffer, RawIOBase, Whence};

const N: usize = 4096;
const STREAM_N: usize = 256 * 1024; // four 64 KiB chunks

fn byte_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("byte_buffer");
    group.throughput(Throughput::Bytes(N as u64));

    let payload = vec![0xABu8; N];
    group.bench_function("pwrite_byte_array", |b| {
        b.iter(|| {
            let mut buf = ByteBuffer::new();
            buf.pwrite_byte_array(0, Whence::Start, black_box(&payload))
                .unwrap();
            buf
        })
    });

    let buf = ByteBuffer::from_bytes(payload.clone());
    group.bench_function("pread_byte_array", |b| {
        b.iter(|| {
            buf.pread_byte_array(0, Whence::Start, black_box(N))
                .unwrap()
        })
    });

    group.finish();
}

fn bit_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("bit_buffer");
    group.throughput(Throughput::Elements(N as u64));

    let bits = vec![true; N];
    group.bench_function("pwrite_bit_array_aligned", |b| {
        b.iter(|| {
            let mut buf = BitBuffer::new();
            buf.pwrite_bit_array(0, Whence::Start, black_box(&bits))
                .unwrap();
            buf
        })
    });

    // Start at bit 3: exercises the head/tail bit path around the packed body.
    group.bench_function("pwrite_bit_array_unaligned", |b| {
        b.iter(|| {
            let mut buf = BitBuffer::new();
            buf.pwrite_bit_array(3, Whence::Start, black_box(&bits))
                .unwrap();
            buf
        })
    });

    let buf = BitBuffer::from_bytes(vec![0xFF; N / 8]);
    group.bench_function("pread_bit_array_aligned", |b| {
        b.iter(|| buf.pread_bit_array(0, Whence::Start, black_box(N)).unwrap())
    });

    group.bench_function("pread_bit_array_unaligned", |b| {
        b.iter(|| {
            buf.pread_bit_array(3, Whence::Start, black_box(N - 8))
                .unwrap()
        })
    });

    group.finish();
}

fn stream(c: &mut Criterion) {
    let mut group = c.benchmark_group("stream");
    group.throughput(Throughput::Bytes(STREAM_N as u64));

    let source = ByteBuffer::from_bytes(vec![0x5Au8; STREAM_N]);
    group.bench_function("pread_raw_io_byte_to_byte", |b| {
        b.iter(|| {
            let mut sink = ByteBuffer::new();
            source
                .pread_raw_io(0, Whence::Start, STREAM_N, &mut sink, 0, Whence::Start)
                .unwrap();
            sink
        })
    });

    group.bench_function("pwrite_raw_io_byte_from_byte", |b| {
        b.iter(|| {
            let mut sink = ByteBuffer::new();
            sink.pwrite_raw_io(0, Whence::Start, &source, 0, Whence::Start, STREAM_N)
                .unwrap();
            sink
        })
    });

    group.finish();
}

criterion_group!(benches, byte_buffer, bit_buffer, stream);
criterion_main!(benches);
