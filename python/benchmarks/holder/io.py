"""Python overhead for describing, buffering, and opening a handle.

Run after ``maturin develop --release`` with::

    python benchmarks/holder/io.py --iterations 10000
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
from yggdryl.holder import Path


PAYLOAD = bytes(range(256)) * 4096
BUFFERED = IOBase.from_bytes(PAYLOAD).buffered(
    page_size=64 * 1024, max_bytes=8 * 1024 * 1024, ttl=30.0
)
APPEND = IOBase.from_bytes()
# UTF-8 content, because the text answer decodes what it reads.
TEXT = IOBase.from_bytes(b"symbol,price\n" * 1024)
ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-io-layers-"))
MEDIA = IOBase(ROOT / "rows.arrows")
MEDIA.overwrite_arrow_table(pa.table({"id": range(4096)}))
MEDIA.open()
# A compound name: three layers to compose and one class to answer with.
COMPOSED = ROOT / "trades.txt.gz"


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
        _measure(
            "buffered page hit",
            lambda: BUFFERED.read_range_bytes(65_000, 128),
            arguments.iterations,
        )
        # The inferring entry point over that same native call: the delta is
        # what the boundary's type dispatch costs, nothing else.
        _measure(
            "range read inferred",
            lambda: BUFFERED.read_range(65_000, 128),
            arguments.iterations,
        )
        _measure(
            "range read as text",
            lambda: TEXT.read_range(0, 128, cls=str),
            arguments.iterations,
        )
        _measure(
            "append bytes",
            lambda: APPEND.append_bytes(b"AAPL,1\n"),
            arguments.iterations,
        )
        _measure(
            "append inferred",
            lambda: APPEND.append("AAPL,1\n"),
            arguments.iterations,
        )
        # What a described handle costs. The role-only spelling builds the
        # same local location and skips the composition, so the delta is what
        # reading the name and answering the class costs - and nothing here
        # touches the store.
        _measure(
            "describe composed",
            lambda: IOBase(COMPOSED),
            arguments.iterations,
        )
        _measure(
            "describe role only",
            lambda: Path(COMPOSED),
            arguments.iterations,
        )
        # Construction plus one descent, against construction alone above.
        _measure(
            "describe and descend",
            lambda: IOBase(COMPOSED).into_handle(),
            arguments.iterations,
        )
        _measure(
            "buffered page cache",
            lambda: BUFFERED.cached_pages,
            arguments.iterations,
        )
        _measure("opened media field", MEDIA.read_arrow_field, arguments.iterations)
        _measure("opened media row size", lambda: MEDIA.row_size, arguments.iterations)
    finally:
        MEDIA.close()
        gc.enable()


if __name__ == "__main__":
    main()
