"""Benchmark yggdryl.converter against Python's native int() / str() / array cast.

The dtype-keyed converter is weighed against the stdlib for the same three
operations: flexibly parsing decimal strings to i32, rendering i32 to strings, and
bulk-casting i32 bytes to i64.

Build the extension in RELEASE first — a debug build is meaningless:

    uv run maturin develop --release
    uv run python bindings/python/benchmarks/bench_converter.py
"""

import array
import time

from yggdryl import converter

N = 100_000
ITERS = 50
VALUES = list(range(N))
STRINGS = [str(i) for i in VALUES]
PARSE_BYTES = sum(len(s) for s in STRINGS)
FORMAT_BYTES = PARSE_BYTES
CAST_SIZE = N * 4  # i32 source bytes
CAST_DATA = array.array("i", VALUES).tobytes()


def throughput_mb_s(nbytes, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return nbytes * iters / secs / (1024 * 1024)


def _ygg_parse():
    for s in STRINGS:
        converter.parse(s, "i32")


def _std_parse():
    for s in STRINGS:
        int(s)


def _ygg_format():
    for v in VALUES:
        converter.format(v, "i32")


def _std_format():
    for v in VALUES:
        str(v)


def _ygg_cast():
    converter.cast(CAST_DATA, "i32", "i64")


def _std_cast():
    # array.array widens i32 -> i64 element-by-element through Python ints.
    src = array.array("i")
    src.frombytes(CAST_DATA)
    array.array("q", src)


def main():
    print(f"yggdryl.converter vs stdlib, {N} values, {ITERS} iters:\n")
    header = f"{'op':>16}  {'yggdryl':>10}  {'stdlib':>10}  {'ratio':>7}"
    print(header)
    print("-" * len(header))
    cases = (
        ("parse->i32", PARSE_BYTES, _ygg_parse, _std_parse),
        ("format i32", FORMAT_BYTES, _ygg_format, _std_format),
        ("cast i32->i64", CAST_SIZE, _ygg_cast, _std_cast),
    )
    for name, nbytes, ygg_op, std_op in cases:
        ygg = throughput_mb_s(nbytes, ITERS, ygg_op)
        std = throughput_mb_s(nbytes, ITERS, std_op)
        print(f"{name:>16}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")


if __name__ == "__main__":
    main()
