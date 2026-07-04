"""Benchmarks for the yggdryl.factory type-inference wrappers.

Measures the per-call cost of inferring a data type from a native value and
building the matching object across the FFI boundary. No dependencies: run with
`python benches/bench_factory.py`.
"""

import time

from yggdryl import factory

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
    bench("factory.scalar(int)", lambda: factory.scalar(42))
    bench("factory.scalar(bytes)", lambda: factory.scalar(b"\x01\x02\x03\x04"))
    bench("factory.scalar(None)", lambda: factory.scalar(None))
    bench("factory.scalar(list[int])", lambda: factory.scalar([1, 2, 3, 4]))
    bench("factory.dtype(int)", lambda: factory.dtype(42))
    bench("factory.field(name, int)", lambda: factory.field("id", 42))


if __name__ == "__main__":
    main()
