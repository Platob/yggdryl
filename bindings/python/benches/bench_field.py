"""Benchmarks for the yggdryl.field Python wrappers.

Measures the per-call cost of the field surface across the FFI boundary
(dominated by pyo3 call overhead; compare with the Rust-side criterion numbers in
crates/yggdryl-field/benches). No dependencies: run with
`python benches/bench_field.py`.
"""

import time

from yggdryl import field

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
    column = field.Int64("id", False)

    bench("Int64('id', False)", lambda: field.Int64("id", False))
    bench("field.name()", column.name)
    bench("field.data_type()", column.data_type)
    bench("field.is_nullable()", column.is_nullable)


if __name__ == "__main__":
    main()
