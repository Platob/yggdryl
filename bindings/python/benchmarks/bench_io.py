"""Benchmark yggdryl.io.ByteBuffer against Python's io.BytesIO, plus streaming gzip.

The Rust-backed ByteBuffer is weighed against the stdlib in-memory stream; the
streaming gzip path is weighed against one-shot stdlib gzip.

Build the extension in RELEASE first — a debug build is meaningless:

    maturin develop --release
    python bindings/python/benchmarks/bench_io.py
"""

import array
import gzip as stdlib_gzip
import io as stdio
import time

from yggdryl import compression
from yggdryl.buffer import I64Buffer
from yggdryl.io import ByteBuffer, I64Cursor, I256Cursor, Whence

SIZE = 1 << 20
ITERS = 200
CHUNK = 64 * 1024
DATA = b"x" * SIZE


def throughput_mb_s(nbytes, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return nbytes * iters / secs / (1024 * 1024)


def _chunks():
    pos = 0
    while pos < SIZE:
        yield pos, min(pos + CHUNK, SIZE)
        pos += CHUNK


def main():
    print(f"ByteBuffer vs io.BytesIO over {SIZE // 1024} KiB, {ITERS} iters:\n")
    header = f"{'op':>10}  {'yggdryl':>10}  {'BytesIO':>10}  {'ratio':>7}"
    print(header)
    print("-" * len(header))

    def ygg_write():
        cursor = ByteBuffer.with_byte_capacity(SIZE).byte_cursor()
        for start, end in _chunks():
            cursor.pwrite_byte_array(DATA[start:end], Whence.Current)

    def bytesio_write():
        buf = stdio.BytesIO()
        for start, end in _chunks():
            buf.write(DATA[start:end])

    src = ByteBuffer(DATA)
    bio = stdio.BytesIO(DATA)
    scratch = bytearray(CHUNK)  # reused — zero per-call allocation

    def ygg_read():
        cursor = src.byte_cursor()
        while cursor.pread_into(scratch, Whence.Current):  # fill-into, no alloc
            pass

    def bytesio_read():
        bio.seek(0)
        while bio.readinto(scratch):
            pass

    for name, ygg_op, std_op in (
        ("write", ygg_write, bytesio_write),
        ("read", ygg_read, bytesio_read),
    ):
        ygg = throughput_mb_s(SIZE, ITERS, ygg_op)
        std = throughput_mb_s(SIZE, ITERS, std_op)
        print(f"{name:>10}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")

    # Overhead check: the type-inferring write() should match the explicit
    # pwrite_byte_array on the bytes fast path (ratio ~1.0).
    def ygg_write_inferred():
        cursor = ByteBuffer.with_byte_capacity(SIZE).byte_cursor()
        for start, end in _chunks():
            cursor.write(DATA[start:end], Whence.Current)

    inferred = throughput_mb_s(SIZE, ITERS, ygg_write_inferred)
    explicit = throughput_mb_s(SIZE, ITERS, ygg_write)
    print(
        f"  write() inferred {inferred:8.1f} MB/s   "
        f"pwrite_byte_array {explicit:8.1f} MB/s   {inferred / explicit:.2f}x (overhead)"
    )

    print("\nTypedCursor<i64> vs array.array('q'):")
    count = SIZE // 8
    values = list(range(count))
    i64_buf = I64Buffer(values)

    def ygg_typed_write():
        cursor = I64Cursor.with_capacity(count)
        cursor.pwrite_array(values, Whence.Start)

    def array_write():
        array.array("q", values).tobytes()

    def ygg_typed_read():
        i64_buf.cursor().pread_array(count, Whence.Start)

    raw = array.array("q", values).tobytes()

    def array_read():
        a = array.array("q")
        a.frombytes(raw)

    for name, ygg_op, std_op in (
        ("write", ygg_typed_write, array_write),
        ("read", ygg_typed_read, array_read),
    ):
        ygg = throughput_mb_s(SIZE, ITERS, ygg_op)
        std = throughput_mb_s(SIZE, ITERS, std_op)
        print(f"{name:>10}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")

    print("\nI256Cursor vs Python int <-> bytes (32-byte values):")
    n256 = SIZE // 32
    values256 = [i for i in range(n256)]

    def ygg_256_write():
        cur = I256Cursor.with_capacity(n256)
        cur.pwrite_array(values256, Whence.Start)

    def py_256_write():
        b"".join(v.to_bytes(32, "little", signed=True) for v in values256)

    raw256 = I256Cursor.with_capacity(n256)
    raw256.pwrite_array(values256, Whence.Start)
    frozen256 = raw256.to_byte_buffer()

    def ygg_256_read():
        I256Cursor.from_bytes(frozen256.as_bytes()).pread_array(n256, Whence.Start)

    raw_bytes = frozen256.as_bytes()

    def py_256_read():
        [
            int.from_bytes(raw_bytes[i : i + 32], "little", signed=True)
            for i in range(0, len(raw_bytes), 32)
        ]

    for name, ygg_op, std_op in (
        ("write", ygg_256_write, py_256_write),
        ("read", ygg_256_read, py_256_read),
    ):
        ygg = throughput_mb_s(SIZE, ITERS, ygg_op)
        std = throughput_mb_s(SIZE, ITERS, std_op)
        print(f"{name:>10}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")

    print("\nByteSlice window read vs memoryview slice:")
    src_buf = ByteBuffer(DATA)

    def ygg_slice_read():
        sl = src_buf.byte_slice(0, SIZE)
        while sl.pread_byte_array(CHUNK, Whence.Current):
            pass

    mv = memoryview(DATA)

    def memoryview_read():
        for pos in range(0, SIZE, CHUNK):
            _ = mv[pos : pos + CHUNK]

    ygg = throughput_mb_s(SIZE, ITERS, ygg_slice_read)
    std = throughput_mb_s(SIZE, ITERS, memoryview_read)
    print(f"{'read':>10}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")

    print("\ngzip level 6 streaming compression:")
    gzip = compression.Gzip(6)

    def ygg_stream():
        gzip.compress_stream(
            ByteBuffer(DATA).byte_cursor(),
            ByteBuffer.with_byte_capacity(SIZE // 2).byte_cursor(),
        )

    def stdlib_oneshot():
        stdlib_gzip.compress(DATA, 6)

    ygg = throughput_mb_s(SIZE, ITERS, ygg_stream)
    std = throughput_mb_s(SIZE, ITERS, stdlib_oneshot)
    print(f"  yggdryl stream {ygg:8.1f} MB/s   stdlib one-shot {std:8.1f} MB/s   {ygg / std:.2f}x")


if __name__ == "__main__":
    main()
