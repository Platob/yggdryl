"""Benchmarks for the yggdryl.factory type-inference wrappers.

Measures the per-call cost of inferring a data type from a native value and
building the matching object across the FFI boundary. No dependencies: run with
`python benches/bench_factory.py`.
"""

import time

from yggdryl import factory, scalar

N = 200_000


def bench(label, function):
    # One warm-up pass, then the timed loop.
    function()
    start = time.perf_counter()
    for _ in range(N):
        function()
    elapsed = time.perf_counter() - start
    print(f"{label:36} {elapsed / N * 1e9:9.1f} ns/op")


def main():
    value = scalar.Int64Scalar(42)
    row = scalar.RecordScalar({"x": 1, "y": 2})

    bench("factory.scalar(int)", lambda: factory.scalar(42))
    bench("factory.scalar(float)", lambda: factory.scalar(1.5))
    bench("factory.scalar(bytes)", lambda: factory.scalar(b"\x01\x02\x03\x04"))
    bench("factory.scalar(None)", lambda: factory.scalar(None))
    bench("factory.scalar(list[int])", lambda: factory.scalar([1, 2, 3, 4]))
    bench("factory.scalar(list[float])", lambda: factory.scalar([1.5, 2.5, 3.5, 4.5]))
    bench("factory.scalar(dict)", lambda: factory.scalar({"x": 1, "y": 2}))
    bench("factory.scalar(scalar object)", lambda: factory.scalar(value))
    bench("factory.dtype(int)", lambda: factory.dtype(42))
    bench("factory.dtype(float)", lambda: factory.dtype(1.5))
    bench("factory.dtype(dict)", lambda: factory.dtype({"x": 1, "y": 2}))
    bench("factory.field(name, int)", lambda: factory.field("id", 42))
    bench("RecordScalar(dict)", lambda: scalar.RecordScalar({"x": 1, "y": 2}))
    bench("record.to_pyvalue()", row.to_pyvalue)
    bench("record.to_pydict()", row.to_pydict)


if __name__ == "__main__":
    main()
