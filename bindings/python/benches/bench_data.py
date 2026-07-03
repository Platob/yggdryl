"""Benchmarks for the yggdryl.data Python wrappers.

Measures the per-call cost of the data-model surface across the FFI boundary
(dominated by pyo3 call overhead; compare with the Rust-side criterion numbers in
crates/yggdryl-data/benches). No dependencies: run with `python benches/bench_data.py`.
"""

import time

from yggdryl import data

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
    int64 = data.Int64Type()
    scalar = data.Int64(42)
    optional = data.OptionalInt64(42)
    encoded = int64.native_to_bytes(42)

    bench("Int64(42)", lambda: data.Int64(42))
    bench("Int64.null()", data.Int64.null)
    bench("scalar.value()", scalar.value)
    bench("scalar.as_i64() direct", scalar.as_i64)
    bench("scalar.as_i8() converted", scalar.as_i8)
    bench("scalar.as_f64() checked", scalar.as_f64)
    bench("OptionalInt64(42)", lambda: data.OptionalInt64(42))
    bench("optional.as_i64() redirected", optional.as_i64)
    bench("optional.data_type()", optional.data_type)
    bench("Int64Type().optional()", int64.optional)
    bench("native_to_bytes(42)", lambda: int64.native_to_bytes(42))
    bench("native_from_bytes(8B)", lambda: int64.native_from_bytes(encoded))
    bench("Int64Field('id', False)", lambda: data.Int64Field("id", False))


if __name__ == "__main__":
    main()
