//! Benchmarks for the `ByteBuffer` and `BitBuffer` resources.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_core::{BitBuffer, ByteBuffer, RawIOBase, Whence};

const N: usize = 4096;

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
    group.bench_function("pwrite_bit_array", |b| {
        b.iter(|| {
            let mut buf = BitBuffer::new();
            buf.pwrite_bit_array(0, Whence::Start, black_box(&bits))
                .unwrap();
            buf
        })
    });

    let buf = BitBuffer::from_bytes(vec![0xFF; N / 8]);
    group.bench_function("pread_bit_array", |b| {
        b.iter(|| buf.pread_bit_array(0, Whence::Start, black_box(N)).unwrap())
    });

    group.finish();
}

criterion_group!(benches, byte_buffer, bit_buffer);
criterion_main!(benches);
