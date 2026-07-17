"""Fast time + memory benchmark for yggdryl.local.LocalIO (runs in ~1-2 s).

Exercises what a caller of the local access point touches: the **lazy auto-create** first
write (parents + file + mapping brought into being on demand), the **self-optimized** mapped
read/write fast path that follows, the **SIMD bulk** typed arrays that delegate to the mapped
backing, the ad-hoc vs mapped read gap, and the **memory-tree** directory read. Time is in
Mops/s; `tracemalloc` reports the Python-heap peak per bulk read.

True multi-thread concurrency is a Rust-core story (Python's GIL serializes these calls) — see
`benchmarks/yggdryl-core/io/local/io.md` for the shared-mapping / disjoint-file scaling.

Build the extension in RELEASE first (a debug build is meaningless for timings; the memory
numbers are build-independent):

    maturin develop --release
    python bindings/python/benchmarks/io/bench_local.py
"""

import os
import tempfile
import time
import tracemalloc

from yggdryl.local import LocalIO

ITERS = 10_000
VALUES = list(range(1024))  # the i32 corpus the bulk rows move


def mops_s(items, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return items * iters / secs / 1_000_000


def peak_bytes_per(count, build):
    kept = build()  # warm one-time state, then discard
    del kept
    tracemalloc.start()
    kept = build()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    del kept
    return peak / count


def main():
    print(f"yggdryl.local.LocalIO - time & memory ({ITERS} iters)\n")

    with tempfile.TemporaryDirectory() as tmp:
        root = LocalIO(tmp)

        # A persistent self-optimized (mapped) handle for the fast-path rows.
        hot = root / "hot.bin"
        hot.pwrite_i32_array(0, VALUES)  # first write maps it
        assert hot.is_mapped

        # A directory memory tree: 16 file blocks of 256 bytes.
        tree = root / "tree"
        for i in range(16):
            block = tree / f"b{i:02d}.bin"
            block.pwrite_byte_array(0, bytes([i]) * 256)
            block.close()

        # A counter so each lazy-create row targets a fresh path (a true *first* write).
        lazy_n = [0]

        def lazy_first_write():
            n = lazy_n[0]
            lazy_n[0] = n + 1
            node = LocalIO(os.path.join(tmp, f"lazy/d{n}/note.bin"))
            node.pwrite_i64(0, 1 << 40)
            node.close()

        def mapped_typed():
            hot.pwrite_i32(64, -1)
            hot.pread_i32(64)

        def bulk_write():
            hot.pwrite_i32_array(0, VALUES)

        def bulk_read():
            hot.pread_i32_array(0, 1024)

        def adhoc_read():
            LocalIO(os.path.join(tmp, "hot.bin")).pread_byte_array(0, 4096)

        def tree_read():
            tree.byte_size()
            tree.pread_byte_array(0, 16 * 256)

        ops = [
            ("lazy first write (mkdir+create+map)", lazy_first_write, 1),
            ("mapped pwrite_i32+pread_i32", mapped_typed, 2),
            ("bulk pwrite_i32_array (1024)", bulk_write, 1024),
            ("bulk pread_i32_array (1024)", bulk_read, 1024),
            ("ad-hoc pread 4 KiB (never written)", adhoc_read, 1),
            ("tree byte_size + pread (16x256)", tree_read, 1),
        ]
        print("time (Mops/s):")
        for name, op, items in ops:
            # Filesystem-bound rows get fewer iterations so the whole run stays ~1-2 s.
            iters = ITERS if items >= 2 else ITERS // 20
            print(f"  {name:<38} {mops_s(items, iters, op):9.3f}")

        # ---- memory ---------------------------------------------------------------------
        n = 20_000
        per_read = peak_bytes_per(n, lambda: [hot.pread_i32_array(0, 256) for _ in range(n)])
        print("\nmemory (Python-heap peak, tracemalloc):")
        print(f"  {'bytes / pread_i32_array(256)':<38} {per_read:9.1f}")

        # Self-check: the mapped bulk round-trip is exact, and the tree stitches its blocks.
        assert hot.pread_i32_array(0, 1024) == VALUES
        assert tree.byte_size() == 16 * 256
        assert tree.pread_byte_array(3, 256)[:1] == bytes([0])  # spans block 0 into block 1

        hot.close()  # release the mapping before the temp dir is cleaned (Windows)


if __name__ == "__main__":
    main()
