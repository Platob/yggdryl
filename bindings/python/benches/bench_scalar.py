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
    blob = scalar.BinaryScalar(b"\x01\x02\x03\x04")
    numbers = scalar.Int64Serie([1, 2, 3, 4])
    weight = scalar.Float64Scalar(1.5)
    weights = scalar.Float64Serie([1.5, 2.5, 3.5, 4.5])
    half = scalar.Float16Scalar(1.5)
    text = scalar.Utf8Scalar("hello")

    bench("Int64Scalar(42)", lambda: scalar.Int64Scalar(42))
    bench("Int64Scalar.null()", scalar.Int64Scalar.null)
    bench("scalar.value()", value.value)
    bench("scalar.as_i64() direct", value.as_i64)
    bench("scalar.as_i8() converted", value.as_i8)
    bench("scalar.as_f64() checked", value.as_f64)
    bench("scalar.to_pyvalue()", value.to_pyvalue)
    bench("OptionalInt64Scalar(42)", lambda: scalar.OptionalInt64Scalar(42))
    bench("optional.as_i64() redirected", optional.as_i64)
    bench("optional.data_type()", optional.data_type)
    bench("optional.to_pyvalue()", optional.to_pyvalue)
    bench("binary.to_pyvalue()", blob.to_pyvalue)
    bench("serie.to_pyvalue()", numbers.to_pyvalue)
    bench("Float64Scalar(1.5)", lambda: scalar.Float64Scalar(1.5))
    bench("float.as_f64() direct", weight.as_f64)
    bench("float.to_pyvalue()", weight.to_pyvalue)
    bench("float serie.to_pyvalue()", weights.to_pyvalue)
    bench("Float16Scalar(1.5)", lambda: scalar.Float16Scalar(1.5))
    bench("float16.as_f16() widened", half.as_f16)
    bench("float16.to_pyvalue()", half.to_pyvalue)
    bench("Utf8Scalar('hello')", lambda: scalar.Utf8Scalar("hello"))
    bench("string.to_pyvalue()", text.to_pyvalue)


if __name__ == "__main__":
    main()
