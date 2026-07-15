"""Fast time + memory benchmark for the yggdryl.types schema layer (runs in ~1 s).

Time: DataType construction + the category drill-down, Field construction with metadata, and
Headers (the centralized metadata map) operations, all reported in Mops/s. Memory: `tracemalloc`
reports the Python-heap peak per DataType / Field / Headers, validating the thin wrapper adds no
runaway allocation.

Build the extension in RELEASE first (a debug build is meaningless for the timings; memory is
build-independent):

    maturin develop --release
    python bindings/python/benchmarks/bench_types.py
"""

import time
import tracemalloc

from yggdryl.io import Headers
from yggdryl.types import DataType, Field

ITERS = 50_000

NAMES = ["u8", "i32", "i64", "u96", "i128", "u256", "f16", "f64", "utf8", "binary"]
TYPES = [DataType.by_name(n) for n in NAMES]


def mops_s(items, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return items * iters / secs / 1_000_000


def peak_bytes_per(count, build):
    kept = build()  # warm any one-time state
    del kept
    tracemalloc.start()
    kept = build()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    del kept
    return peak / count


def main():
    print(f"yggdryl.types - time & memory ({ITERS} iters)\n")

    # ---- time -----------------------------------------------------------------------
    def build_types():
        for n in NAMES:
            DataType.by_name(n)

    def drill_down():
        for dt in TYPES:
            dt.is_integer() or dt.is_floating() or dt.is_utf8()

    def build_fields():
        for dt in TYPES:
            Field("col", dt, False)

    def build_fields_meta():
        for dt in TYPES:
            Field("col", dt, False, {"unit": "count", "source": "x"})

    def metadata_ops():
        h = Headers()
        h["a"] = "1"
        h["b"] = "2"
        _ = h.get("a"), "b" in h, h.items()

    ops = [
        ("DataType.by_name", build_types, len(NAMES)),
        ("category drill-down", drill_down, len(TYPES)),
        ("Field (no metadata)", build_fields, len(TYPES)),
        ("Field (with metadata)", build_fields_meta, len(TYPES)),
        ("Headers build+read", metadata_ops, 1),
    ]
    print("time (Mops/s):")
    for name, op, items in ops:
        print(f"  {name:<26} {mops_s(items, ITERS, op):7.2f}")

    # ---- memory ---------------------------------------------------------------------
    n = 20_000
    per_dt = peak_bytes_per(n, lambda: [DataType.by_name(NAMES[i % len(NAMES)]) for i in range(n)])
    per_field = peak_bytes_per(n, lambda: [Field("c", TYPES[i % len(TYPES)], False) for i in range(n)])
    per_meta = peak_bytes_per(n, lambda: [Headers({"k": str(i)}) for i in range(n)])
    print("\nmemory (Python-heap peak, tracemalloc):")
    print(f"  {'bytes / DataType':<26} {per_dt:7.1f}")
    print(f"  {'bytes / Field':<26} {per_field:7.1f}")
    print(f"  {'bytes / Headers':<26} {per_meta:7.1f}")


if __name__ == "__main__":
    main()
