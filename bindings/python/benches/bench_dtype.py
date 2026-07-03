"""Benchmarks for the yggdryl.dtype Python wrappers.

Measures the per-call cost of the data-type surface across the FFI boundary
(dominated by pyo3 call overhead; compare with the Rust-side criterion numbers in
crates/yggdryl-dtype/benches). No dependencies: run with
`python benches/bench_dtype.py`.
"""

import time

from yggdryl import dtype

N = 200_000


def bench(label, function):
    # One warm-up pass, then the timed loop.
    function()
    start = time.perf_counter()
    for _ in range(N):
        function()
    elapsed = time.perf_counter() - start
    print(f"{label:32} {elapsed / N * 1e9:9.1f} ns/op")


def main():
    int64 = dtype.Int64Type()
    encoded = int64.native_to_bytes(42)

    bench("Int64Type()", dtype.Int64Type)
    bench("native_to_bytes(42)", lambda: int64.native_to_bytes(42))
    bench("native_from_bytes(8B)", lambda: int64.native_from_bytes(encoded))
    bench("default_value()", int64.default_value)
    bench("default_scalar()", int64.default_scalar)
    bench("field('id')", lambda: int64.field("id"))
    bench("scalar(42)", lambda: int64.scalar(42))
    bench("Int64Type().optional()", int64.optional)


if __name__ == "__main__":
    main()
