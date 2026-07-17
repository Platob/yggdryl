"""Fast time + memory benchmark for yggdryl.memory.Heap (runs in ~1-2 s).

Time: the positioned typed reads (`pread_i32` / `pread_i64` / `pread_byte`), the bulk
`pread_byte_array`, the cursor `write` loop, `slice`, and `from-bytes` ingest are reported in
Mops/s — yggdryl-only ops with no single stdlib equivalent. Memory: `tracemalloc` reports the
Python-heap peak per `Heap(bytes)` ingest and per `pread_byte_array`, validating that the thin
wrapper adds no runaway allocation.

Build the extension in RELEASE first — a debug build is meaningless for the timings (the
memory numbers are build-independent):

    maturin develop --release
    python bindings/python/benchmarks/bench_memory.py
"""

import time
import tracemalloc

from yggdryl.memory import Heap, Whence

ITERS = 10_000

# A packed record: byte + i32 + i64, repeated — the corpus the typed reads walk.
RECORD = b"\x7f" + (-7).to_bytes(4, "little", signed=True) + (1 << 40).to_bytes(8, "little")
BLOB = RECORD * 64  # 832 bytes
CHUNKS = [b"chunk-%04d;" % i for i in range(64)]


def mops_s(items, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return items * iters / secs / 1_000_000


def peak_bytes_per(count, build):
    """Peak traced Python-heap bytes while `count` objects built by `build` are alive."""
    gc_objs = build()  # warm any one-time state, then discard
    del gc_objs
    tracemalloc.start()
    kept = build()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    del kept
    return peak / count


def main():
    print(f"yggdryl.memory.Heap - time & memory ({ITERS} iters)\n")

    heap = Heap(BLOB)
    offsets = list(range(0, len(BLOB) - 13, 13))  # one per record

    def typed_reads():
        for off in offsets:
            heap.pread_byte(off)
            heap.pread_i32(off + 1)
            heap.pread_i64(off + 5)

    def bulk_read():
        for off in offsets:
            heap.pread_byte_array(off, 13)

    def cursor_write():
        h = Heap.with_capacity(len(BLOB))
        for chunk in CHUNKS:
            h.write(chunk)

    def slice_windows():
        for off in offsets:
            heap.slice(off, 13)

    def ingest():
        Heap(BLOB)

    # ---- time -----------------------------------------------------------------------
    ops = [
        ("typed reads (byte+i32+i64)", typed_reads, len(offsets) * 3),
        ("pread_byte_array", bulk_read, len(offsets)),
        ("cursor write", cursor_write, len(CHUNKS)),
        ("slice", slice_windows, len(offsets)),
        ("from-bytes ingest", ingest, 1),
    ]
    print("time (Mops/s):")
    for name, op, items in ops:
        print(f"  {name:<28} {mops_s(items, ITERS, op):7.2f}")

    # ---- memory ---------------------------------------------------------------------
    n = 20_000
    per_heap = peak_bytes_per(n, lambda: [Heap(RECORD) for _ in range(n)])
    per_read = peak_bytes_per(n, lambda: [heap.pread_byte_array(0, 13) for _ in range(n)])
    print("\nmemory (Python-heap peak, tracemalloc):")
    print(f"  {'bytes / Heap(bytes) ingest':<28} {per_heap:7.1f}")
    print(f"  {'bytes / pread_byte_array':<28} {per_read:7.1f}")

    # A quick self-check the ops actually did the round-trip they claim.
    assert heap.pread_i64(5) == 1 << 40
    assert Heap(BLOB).slice(0, 13).to_bytes() == RECORD
    assert Heap(b"seek").seek(Whence.End, 0) == 4


if __name__ == "__main__":
    main()
