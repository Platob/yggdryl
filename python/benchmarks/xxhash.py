"""The digest boundary against C ``libxxhash``, same protocol, same payload.

Every row is a like-for-like engine comparison: the ``xxhash`` package binds C
``libxxhash``, so a row pairs the same algorithm over the same bytes and the
difference is the engine plus this boundary's conversion cost. The conversion
is deliberately visible rather than averaged away - the ``bytearray`` and
``memoryview`` rows are what a caller pays for a buffer that is not ``bytes``,
and the small-payload rows are where a call's fixed cost still shows.

The final rows measure Arrow holder filling through the Python boundary. They
separate schema insertion, default recomputation, preservation, and ``force``;
the fixture and protocol metadata are built outside every measured call.

The file is named for the topic rather than the module, as ``compression.py``
is: a benchmark named ``xxhash.py`` would shadow the C ``xxhash`` package it
compares against, because a script's own directory comes first on ``sys.path``.

Usage:
    python benchmarks/xxhash.py [--min-time 0.2] [--repeat 5]
"""

from __future__ import annotations

import argparse
import statistics
import sys
import timeit

import pyarrow as pa

from yggdryl import DataType, Field, Scalar, xxhash

try:
    import xxhash as xxhash_c
except ImportError:  # pragma: no cover - the comparison is optional
    xxhash_c = None

PAYLOAD = b'{"id": 1234567, "venue": "XNAS", "price": "150.2500"}\n' * 20_000

#: The sizes where a call's fixed cost, the size branches, and the streaming
#: kernel each dominate in turn.
SIZES = [1, 4, 16, 64, 128, 240, 1024, 64 * 1024, 1024 * 1024]

#: Large enough for per-row work to dominate one Python call.
BATCH_ROWS = 4_096


def _measure(label: str, callable_, size: int, min_time: float, repeat: int) -> None:
    timer = timeit.Timer(callable_)
    number, _ = timer.autorange()
    while timer.timeit(number) < min_time:
        number *= 2
    samples = [timer.timeit(number) / number for _ in range(repeat)]
    median = statistics.median(samples)
    throughput = size / median / (1000 * 1000 * 1000)
    print(f"{label:46s} {median * 1e9:12.1f} ns {throughput:8.2f} GB/s")


def _measure_rows(label: str, callable_, rows: int, min_time: float, repeat: int) -> None:
    timer = timeit.Timer(callable_)
    number, _ = timer.autorange()
    while timer.timeit(number) < min_time:
        number *= 2
    samples = [timer.timeit(number) / number for _ in range(repeat)]
    median = statistics.median(samples)
    throughput = rows / median / 1_000_000
    print(f"{label:46s} {median * 1e6:12.1f} us {throughput:8.2f} M row/s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()

    print(f"Python {sys.version.split()[0]}; median of {arguments.repeat}, "
          f"min {arguments.min_time}s per sample")

    cases: list[tuple[str, object, int]] = []

    # One-shot, per size: where the dispatch cost stops mattering.
    for size in SIZES:
        data = PAYLOAD[:size] if size <= len(PAYLOAD) else PAYLOAD * (size // len(PAYLOAD) + 1)
        data = data[:size]
        cases.append((f"xxh3 {size:>9} B", lambda data=data: xxhash.xxh3(data), size))
        if xxhash_c is not None:
            cases.append(
                (
                    f"xxh3 {size:>9} B (C libxxhash)",
                    lambda data=data: xxhash_c.xxh3_64_intdigest(data),
                    size,
                )
            )

    size = len(PAYLOAD)
    # Every algorithm at one size, so the widths compare directly.
    for name, native in (
        ("xxh32", xxhash.xxh32),
        ("xxh64", xxhash.xxh64),
        ("xxh3", xxhash.xxh3),
        ("xxh128", xxhash.xxh128),
    ):
        cases.append((f"{name} payload", lambda native=native: native(PAYLOAD), size))
    if xxhash_c is not None:
        for name, outside in (
            ("xxh32", xxhash_c.xxh32_intdigest),
            ("xxh64", xxhash_c.xxh64_intdigest),
            ("xxh3", xxhash_c.xxh3_64_intdigest),
            ("xxh128", xxhash_c.xxh3_128_intdigest),
        ):
            cases.append(
                (f"{name} payload (C libxxhash)", lambda outside=outside: outside(PAYLOAD), size)
            )

    # The conversion cost, made visible: `bytes` is borrowed, every other
    # buffer is read through one bounded window.
    mutable = bytearray(PAYLOAD)
    view = memoryview(PAYLOAD)
    text = PAYLOAD.decode()
    cases.append(("xxh3 payload (bytearray)", lambda: xxhash.xxh3(mutable), size))
    cases.append(("xxh3 payload (memoryview)", lambda: xxhash.xxh3(view), size))
    cases.append(("xxh3 payload (str)", lambda: xxhash.xxh3(text), size))

    # Streaming against one-shot, and the digest wrapper against the bare int.
    def streamed() -> int:
        state = xxhash.Xxh3()
        for index in range(0, len(PAYLOAD), 64 * 1024):
            state.write_bytes(view[index : index + 64 * 1024])
        return int(state.as_digest())

    cases.append(("xxh3 payload (streamed 64 KiB)", streamed, size))
    cases.append(
        ("digest payload (Digest wrapper)", lambda: xxhash.digest(PAYLOAD, "xxh3-64"), size)
    )

    # The value feed: a leaf, a wide record, and the row a table hashes.
    leaf = Scalar.from_py("AAPL")
    record = Scalar.from_py({f"column_{index:03}": index for index in range(64)})
    cases.append(("scalar leaf digest", lambda: leaf.digest(), 4))
    cases.append(("scalar wide record digest", lambda: record.digest(), 64))
    cases.append(("scalar leaf stable_hash", lambda: leaf.stable_hash(), 4))

    for label, callable_, case_size in cases:
        _measure(label, callable_, case_size, arguments.min_time, arguments.repeat)

    symbols = pa.array(
        ["AAPL" if index % 2 == 0 else "MSFT" for index in range(BATCH_ROWS)]
    )
    holder = Field("row_digest", "uint64", nullable=False)
    holder.digest["role"] = "holder"
    holder.digest["paths"] = '["symbol"]'
    root = Field(
        "row",
        DataType.from_fields([Field("symbol", "utf8", nullable=False), holder]),
        nullable=False,
    )
    missing = pa.record_batch([symbols], names=["symbol"])
    defaults = pa.record_batch(
        [symbols, pa.array([0] * BATCH_ROWS, type=pa.uint64())],
        names=["symbol", "row_digest"],
    )
    populated = pa.record_batch(
        [symbols, pa.array(range(1, BATCH_ROWS + 1), type=pa.uint64())],
        names=["symbol", "row_digest"],
    )
    state = xxhash.Xxh3(seed=7)
    fill_cases = [
        ("fill batch (missing holder)", lambda: state.fill_arrow_batch(root, missing)),
        ("fill batch (default holders)", lambda: state.fill_arrow_batch(root, defaults)),
        ("fill batch (preserve populated)", lambda: state.fill_arrow_batch(root, populated)),
        (
            "fill batch (force populated)",
            lambda: state.fill_arrow_batch(root, populated, force=True),
        ),
    ]
    for label, callable_ in fill_cases:
        _measure_rows(label, callable_, BATCH_ROWS, arguments.min_time, arguments.repeat)

    if xxhash_c is None:
        print("the xxhash package (C libxxhash) is not installed; comparison rows skipped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
