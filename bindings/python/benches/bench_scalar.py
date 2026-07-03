"""Benchmarks for the yggdryl.scalar Python wrappers.

Measures the per-call cost of the scalar surface across the FFI boundary
(dominated by pyo3 call overhead; compare with the Rust-side criterion numbers in
crates/yggdryl-scalar/benches). No dependencies: run with
`python benches/bench_scalar.py`.
"""

import time

from yggdryl import scalar

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
    value = scalar.Int64Scalar(42)
    optional = scalar.OptionalInt64Scalar(42)

    bench("Int64Scalar(42)", lambda: scalar.Int64Scalar(42))
    bench("Int64Scalar.null()", scalar.Int64Scalar.null)
    bench("scalar.value()", value.value)
    bench("scalar.as_i64() direct", value.as_i64)
    bench("scalar.as_i8() converted", value.as_i8)
    bench("scalar.as_f64() checked", value.as_f64)
    bench("OptionalInt64Scalar(42)", lambda: scalar.OptionalInt64Scalar(42))
    bench("optional.as_i64() redirected", optional.as_i64)
    bench("optional.data_type()", optional.data_type)


if __name__ == "__main__":
    main()
