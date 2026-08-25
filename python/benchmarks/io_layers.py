"""Python overhead for generic buffered and opened-media redirection.

Run after ``maturin develop --release`` with::

    python benchmarks/io_layers.py --iterations 10000
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import statistics
import tempfile
import timeit
from collections.abc import Callable

import pyarrow as pa

from yggdryl import IOBase


PAYLOAD = bytes(range(256)) * 4096
BUFFERED = IOBase.from_bytes(PAYLOAD).buffered(
    page_size=64 * 1024, max_bytes=8 * 1024 * 1024, ttl=30.0
)
ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-io-layers-"))
MEDIA = IOBase(ROOT / "rows.arrows")
MEDIA.overwrite_arrow_table(pa.table({"id": range(4096)}))
MEDIA.open()


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    nanoseconds = statistics.median(samples) * 1_000_000_000 / iterations
    print(f"{name:31} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    arguments = parser.parse_args()
    if arguments.iterations < 1:
        parser.error("--iterations must be positive")

    gc.disable()
    try:
        _measure("buffered page hit", lambda: BUFFERED.pread(65_000, 128), arguments.iterations)
        _measure(
            "buffered idempotent redirect",
            lambda: BUFFERED.buffered(
                page_size=64 * 1024, max_bytes=8 * 1024 * 1024, ttl=30.0
            ),
            arguments.iterations,
        )
        _measure("opened media field", MEDIA.read_arrow_field, arguments.iterations)
        _measure("opened media row size", lambda: MEDIA.row_size, arguments.iterations)
    finally:
        MEDIA.close()
        gc.enable()


if __name__ == "__main__":
    main()
