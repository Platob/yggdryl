"""Benchmark yggdryl.buffer.I32Buffer against Python's array.array.

The Rust-backed typed buffer is weighed against the stdlib's contiguous typed
array for the same three operations: constructing from a list of values,
serialising to bytes, and deserialising from bytes.

Build the extension in RELEASE first — a debug build is meaningless:

    uv run maturin develop --release
    uv run python bindings/python/benchmarks/bench_buffer.py
"""

import array
import time

from yggdryl.buffer import I32Buffer

COUNT = (1 << 20) // 4  # 256 Ki i32 == 1 MiB
SIZE = COUNT * 4
ITERS = 200
VALUES = list(range(COUNT))
DATA = array.array("i", VALUES).tobytes()


def throughput_mb_s(nbytes, iters, op):
    op()  # warm up
    start = time.perf_counter()
    for _ in range(iters):
        op()
    secs = time.perf_counter() - start
    return nbytes * iters / secs / (1024 * 1024)


def _array_from_bytes():
    a = array.array("i")
    a.frombytes(DATA)
    return a


def main():
    print(f"I32Buffer vs array.array over {SIZE // 1024} KiB ({COUNT} i32), {ITERS} iters:\n")
    header = f"{'op':>12}  {'yggdryl':>10}  {'array':>10}  {'ratio':>7}"
    print(header)
    print("-" * len(header))

    # pre-built inputs so each row measures one pure operation
    ygg_buf = I32Buffer(VALUES)
    std_arr = array.array("i", VALUES)
    cases = (
        ("construct", lambda: I32Buffer(VALUES), lambda: array.array("i", VALUES)),
        ("serialize", ygg_buf.serialize_bytes, std_arr.tobytes),
        ("deserialize", lambda: I32Buffer.deserialize_bytes(DATA), _array_from_bytes),
    )
    for name, ygg_op, std_op in cases:
        ygg = throughput_mb_s(SIZE, ITERS, ygg_op)
        std = throughput_mb_s(SIZE, ITERS, std_op)
        print(f"{name:>12}  {ygg:>8.1f}MB  {std:>8.1f}MB  {ygg / std:>6.2f}x")


if __name__ == "__main__":
    main()
