"""Benchmarks for the yggdryl serie wrappers.

Measures the per-call cost of the serie surface across the FFI boundary
(dominated by pyo3 call overhead plus the element-list conversion; compare with
the Rust-side criterion numbers in crates/yggdryl-scalar/benches/serie.rs). No
dependencies: run with `python benches/bench_serie.py`.
"""

import time

from yggdryl import dtype, scalar

N = 200_000
ELEMENTS = list(range(64))  # one small serie per call, so the loop stays per-call


def bench(label, function):
    # One warm-up pass, then the timed loop.
    function()
    start = time.perf_counter()
    for _ in range(N):
        function()
    elapsed = time.perf_counter() - start
    print(f"{label:32} {elapsed / N * 1e9:9.1f} ns/op")


def main():
    numbers = scalar.Int64Serie(ELEMENTS)
    narrow = scalar.Int8Serie(ELEMENTS)
    serie_type = dtype.Int64SerieType()

    bench("Int64Serie(64 ints)", lambda: scalar.Int64Serie(ELEMENTS))
    bench("Int8Serie(64 ints)", lambda: scalar.Int8Serie(ELEMENTS))
    bench("Int64Serie.null()", scalar.Int64Serie.null)
    bench("serie.len()", numbers.len)
    bench("serie.to_pylist() copy-out", numbers.to_pylist)
    bench("serie.value_at(32)", lambda: numbers.value_at(32))
    bench("serie.scalar_at(32)", lambda: numbers.scalar_at(32))
    bench("narrow.value_at(32)", lambda: narrow.value_at(32))
    bench("SerieType().scalar(64 ints)", lambda: serie_type.scalar(ELEMENTS))
    bench("SerieType().native_to_bytes", lambda: serie_type.native_to_bytes(ELEMENTS))


if __name__ == "__main__":
    main()
