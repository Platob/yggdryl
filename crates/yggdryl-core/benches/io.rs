//! Throughput benchmark for cursor IO ([`ByteCursor`](yggdryl_core::ByteCursor))
//! and, when `gzip` is enabled, streaming compression vs the one-shot codec.
//!
//! Dependency-free (`harness = false`, a plain `main`). Run with
//! `cargo bench -p yggdryl-core`.

use std::time::Instant;

use yggdryl_core::{ByteBuffer, IOBase, IoPrimitive, TypedIOBase, Whence};

/// Runs `op` `iters` times, returning MB/s over `bytes` processed per iteration.
fn throughput_mb_s(bytes: usize, iters: u32, mut op: impl FnMut()) -> f64 {
    op(); // warm up
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    (bytes as f64 * f64::from(iters)) / secs / (1024.0 * 1024.0)
}

fn main() {
    let size = 1 << 20; // 1 MiB
    let iters = 200;
    let chunk = 64 * 1024;
    let data = vec![b'x'; size];

    println!(
        "ByteCursor IO over {} KiB, {iters} iters (64 KiB chunks):",
        size / 1024
    );

    let write = throughput_mb_s(size, iters, || {
        let mut cursor = ByteBuffer::with_byte_capacity(size).byte_cursor();
        let mut pos = 0usize;
        while pos < size {
            let end = (pos + chunk).min(size);
            cursor
                .pwrite_byte_array(&data[pos..end], Whence::Current)
                .unwrap();
            pos = end;
        }
    });

    let source = ByteBuffer::from_bytes(&data);
    let read_alloc = throughput_mb_s(size, iters, || {
        let mut cursor = source.byte_cursor();
        loop {
            let got = cursor.pread_byte_array(chunk, Whence::Current).unwrap();
            if got.is_empty() {
                break;
            }
        }
    });

    // The allocation-free read path used by streaming: fill a reused scratch buffer.
    let mut scratch = vec![0u8; chunk];
    let read_into = throughput_mb_s(size, iters, || {
        let mut cursor = source.byte_cursor();
        while cursor.pread_into(&mut scratch, Whence::Current).unwrap() != 0 {}
    });

    println!(
        "  write {write:9.1} MB/s   read(alloc) {read_alloc:9.1} MB/s   read(into) {read_into:9.1} MB/s"
    );

    typed_bench(size, iters);
    typed_cursor_bench(size, iters);
    wide_int_bench(size, iters);
    slice_bench(&data, iters);
    transfer_bench(&data, iters);
    arrow_bench(size, iters);

    #[cfg(feature = "gzip")]
    stream_bench(&data, iters);
}

/// Zero-copy Arrow wrap vs a copying construction over a 1 MiB payload.
fn arrow_bench(size: usize, iters: u32) {
    use yggdryl_core::arrow_buffer::Buffer;
    use yggdryl_core::ByteBuffer;

    let data = vec![b'x'; size];
    let arrow = Buffer::from_vec(data.clone());

    let wrap = throughput_mb_s(size, iters, || {
        let _ = ByteBuffer::from_arrow_byte_buffer(arrow.clone());
    });
    let copy = throughput_mb_s(size, iters, || {
        let _ = ByteBuffer::from_bytes(&data);
    });
    println!("  arrow from_arrow (zero-copy) {wrap:9.1} MB/s   from_bytes (copy) {copy:9.1} MB/s");
}

/// Typed `i64` array throughput over a 1 MiB payload.
fn typed_bench(size: usize, iters: u32) {
    let count = size / 8;
    let values: Vec<i64> = (0..count as i64).collect();

    let write_i64 = throughput_mb_s(size, iters, || {
        let mut cursor = ByteBuffer::with_byte_capacity(size).byte_cursor();
        cursor.pwrite_i64_array(&values, Whence::Start).unwrap();
    });

    let mut source = ByteBuffer::with_byte_capacity(size).byte_cursor();
    source.pwrite_i64_array(&values, Whence::Start).unwrap();
    let frozen = source.to_byte_buffer();
    let read_i64 = throughput_mb_s(size, iters, || {
        frozen
            .byte_cursor()
            .pread_i64_array(count, Whence::Start)
            .unwrap();
    });

    println!("  i64_array  write {write_i64:9.1} MB/s   read {read_i64:9.1} MB/s");
}

/// Typed `i64` throughput through the element-typed `TypedCursor<i64>` (native
/// `i64` units) over a 1 MiB payload — the typed-cursor counterpart of `typed_bench`.
fn typed_cursor_bench(size: usize, iters: u32) {
    let count = size / 8;
    let values: Vec<i64> = (0..count as i64).collect();

    let write = throughput_mb_s(size, iters, || {
        let mut cursor = <yggdryl_core::TypedCursor<i64> as TypedIOBase<i64>>::with_capacity(count);
        cursor.pwrite_array(&values, Whence::Start).unwrap();
    });

    // Freeze once to a byte buffer; each read wraps it in a fresh cursor (cheap —
    // an `Arc` bump), isolating the read cost from serialisation.
    let mut frozen_bytes = Vec::new();
    for &value in &values {
        value.write_le(&mut frozen_bytes);
    }
    let frozen = ByteBuffer::from_vec(frozen_bytes);
    let read = throughput_mb_s(size, iters, || {
        yggdryl_core::TypedCursor::<i64>::new(frozen.clone())
            .pread_array(count, Whence::Start)
            .unwrap();
    });

    println!("  TypedCursor<i64>  write {write:9.1} MB/s   read {read:9.1} MB/s");
}

/// Wide-integer (`i256`, 32 bytes each) typed-cursor throughput over a 1 MiB payload.
fn wide_int_bench(size: usize, iters: u32) {
    use yggdryl_core::{i256, TypedCursor, TypedIOBase};

    let count = size / 32;
    let values: Vec<i256> = (0..count as i128).map(i256::from_i128).collect();

    let write = throughput_mb_s(size, iters, || {
        let mut cursor = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(count);
        cursor.pwrite_array(&values, Whence::Start).unwrap();
    });

    let frozen = {
        let mut c = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(count);
        c.pwrite_array(&values, Whence::Start).unwrap();
        c.to_byte_buffer()
    };
    let read = throughput_mb_s(size, iters, || {
        TypedCursor::<i256>::new(frozen.clone())
            .pread_array(count, Whence::Start)
            .unwrap();
    });

    println!("  TypedCursor<i256>  write {write:9.1} MB/s   read {read:9.1} MB/s");
}

/// Bounded-window (`ByteSlice`) read/write throughput vs the full-resource
/// [`ByteCursor`], over a 1 MiB window. A slice should carry no measurable overhead
/// beyond its clamping.
fn slice_bench(data: &[u8], iters: u32) {
    use yggdryl_core::{IOSlice, TypedIOBase};

    let size = data.len();
    let source = ByteBuffer::from_bytes(data);

    let read = throughput_mb_s(size, iters, || {
        let mut slice = source.byte_slice(0, size);
        while !slice
            .pread_byte_array(64 * 1024, Whence::Current)
            .unwrap()
            .is_empty()
        {}
    });

    // Typed-slice read of the whole window as `i64`.
    let count = size / 8;
    let read_i64 = throughput_mb_s(size, iters, || {
        let mut slice = source.slice::<i64>(0, size);
        let _ = slice.pread_array(count, Whence::Start).unwrap();
        // touch the bounds so the window accessors are exercised.
        debug_assert_eq!(slice.slice_len(), size);
    });

    println!("  ByteSlice read {read:9.1} MB/s   TypedSlice<i64> read {read_i64:9.1} MB/s");
}

/// Resource-to-resource transfer throughput (`pread_io` between two cursors).
fn transfer_bench(data: &[u8], iters: u32) {
    let size = data.len();
    let source = ByteBuffer::from_bytes(data);
    let transfer = throughput_mb_s(size, iters, || {
        let mut src = source.byte_cursor();
        let mut sink = ByteBuffer::with_byte_capacity(size).byte_cursor();
        src.pread_io(&mut sink, size, Whence::Start).unwrap();
    });
    println!("  pread_io (cursor -> cursor) {transfer:9.1} MB/s");
}

/// Compares streaming compression through cursors against the one-shot codec.
#[cfg(feature = "gzip")]
fn stream_bench(data: &[u8], iters: u32) {
    use yggdryl_core::{CompressionDecoder, CompressionEncoder, Decoder, Encoder, Gzip};

    let gzip = Gzip::new(6).unwrap();
    let size = data.len();
    let source_buf = ByteBuffer::from_bytes(data);
    let compressed_buf = ByteBuffer::from_bytes(&gzip.encode_byte_array(data).unwrap());

    let stream_c = throughput_mb_s(size, iters, || {
        let mut source = source_buf.byte_cursor();
        let mut sink = ByteBuffer::with_byte_capacity(size / 2).byte_cursor();
        gzip.compress_stream(&mut source, &mut sink).unwrap();
    });
    let oneshot_c = throughput_mb_s(size, iters, || {
        gzip.encode_byte_array(data).unwrap();
    });
    let stream_d = throughput_mb_s(size, iters, || {
        let mut source = compressed_buf.byte_cursor();
        let mut sink = ByteBuffer::with_byte_capacity(size).byte_cursor();
        gzip.decompress_stream(&mut source, &mut sink).unwrap();
    });
    let oneshot_d = throughput_mb_s(size, iters, || {
        gzip.decode_byte_array(compressed_buf.as_bytes()).unwrap();
    });

    println!("gzip level 6, streaming (cursor) vs one-shot:");
    println!("  compress   stream {stream_c:9.1} MB/s   one-shot {oneshot_c:9.1} MB/s");
    println!("  decompress stream {stream_d:9.1} MB/s   one-shot {oneshot_d:9.1} MB/s");
}
