"""Time the Arrow line projection against a plain-Python parsing loop.

Run from ``python/`` after ``maturin develop --release`` (a debug build
understates the native side by an order of magnitude)::

    .venv/Scripts/python benchmarks/read_lines.py --min-time 0.2 --repeat 7

The payload is 50k anonymized production-shaped OMS log lines -
``2026-08-14 00:05:01.167_250 [250-<hex>:<hex>:72503] [OrderFlow_Enrichment]
(DEBUG) message`` - timed uncompressed and gzip-coded. The baseline is what a
Python engineer would write without the binding: ``re`` with named groups over
``str.splitlines``, timestamps through ``datetime.fromisoformat``, columns
accumulated into lists and handed to ``pyarrow.table``. **The regex is the
same string on both sides**: ``(?P<name>...)`` groups, which CPython's ``re``
and the binding's engine both read, so the row is an engine comparison over
one expression. The baseline hashes messages with ``zlib.crc32`` - a C-speed
32-bit checksum - while the binding pays for the stable 64-bit FNV-1a the
``hash`` column contractually carries, so the comparison flatters the baseline
slightly and is still a fair "same job" measurement.

The boundary measured is the Arrow C Stream interface: the reader crosses
lazily and every batch is drained on the Python side.
"""

from __future__ import annotations

import argparse
import gc
import gzip
import pathlib
import platform
import re
import shutil
import statistics
import tempfile
import sys
import timeit
import zlib
from dataclasses import dataclass
from datetime import datetime
from typing import Callable

import pyarrow as pa

from yggdryl import IOBase

LINES = 50_000

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _corpus import PATTERN, line as _line  # noqa: E402

BASELINE_PATTERN = re.compile(PATTERN)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-read-lines-"))
PLAIN = ROOT / "bench.log"
CODED = ROOT / "bench.log.gz"

EPOCH = datetime(1970, 1, 1)


def _corpus() -> str:
    return "".join(_line(index) for index in range(LINES))


def _read_lines(target: pathlib.Path) -> int:
    rows = 0
    reader = IOBase(target).read_arrow_lines(PATTERN)
    for batch in reader:
        rows += batch.num_rows
    return rows


def _baseline(target: pathlib.Path) -> int:
    if target.suffix == ".gz":
        text = gzip.decompress(target.read_bytes()).decode()
    else:
        text = target.read_text()
    names = (
        "rownum", "unix", "hash", "header", "message",
        "stamp", "thread", "port", "logger", "level",
    )
    columns: dict[str, list] = {name: [] for name in names}
    rownum = 0
    for line in text.splitlines():
        matched = BASELINE_PATTERN.match(line)
        rownum += 1
        if matched is None:
            continue
        message = line[matched.end() :].strip()
        stamp = datetime.fromisoformat(matched.group("stamp").replace("_", ""))
        columns["rownum"].append(rownum)
        columns["unix"].append(int((stamp - EPOCH).total_seconds() * 1_000_000) * 1_000)
        columns["hash"].append(zlib.crc32(message.encode()))
        columns["header"].append(matched.group(0))
        columns["message"].append(message)
        columns["stamp"].append(matched.group("stamp"))
        columns["thread"].append(matched.group("thread"))
        columns["port"].append(int(matched.group("port")))
        columns["logger"].append(matched.group("logger"))
        columns["level"].append(matched.group("level"))
    return pa.table(columns).num_rows


@dataclass(frozen=True)
class Benchmark:
    """One measured operation and the unit its throughput is reported in."""

    name: str
    operation: Callable[[], object]
    units: int
    unit_name: str


BENCHMARKS = (
    Benchmark("read_arrow_lines plain", lambda: _read_lines(PLAIN), LINES, "row"),
    Benchmark("read_arrow_lines gzip", lambda: _read_lines(CODED), LINES, "row"),
    Benchmark("python re loop plain", lambda: _baseline(PLAIN), LINES, "row"),
    Benchmark("python re loop gzip", lambda: _baseline(CODED), LINES, "row"),
)


def _measure(
    benchmark: Benchmark,
    *,
    minimum_seconds: float,
    repeat: int,
) -> tuple[float, float, int]:
    benchmark.operation()
    number = 1
    while number < 4_096:
        if timeit.timeit(benchmark.operation, number=number) >= minimum_seconds:
            break
        number *= 2
    gc.collect()
    samples = timeit.repeat(benchmark.operation, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def _rate(units: int, seconds: float, unit_name: str) -> str:
    return f"{units / seconds:,.0f} {unit_name}s/s"


def main() -> None:
    """Write the fixtures, then time every parser over them."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=7)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    try:
        text = _corpus()
        PLAIN.write_text(text)
        CODED.write_bytes(gzip.compress(text.encode()))
        # A parser that silently dropped rows would still look fast, so the
        # row counts are asserted before anything is timed.
        assert _read_lines(PLAIN) == LINES
        assert _read_lines(CODED) == LINES
        assert _baseline(PLAIN) == LINES

        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{LINES:,} log lines, {len(text):,} decoded bytes; "
            f"one shared pattern compiled by both engines"
        )
        print(f"{'benchmark':32} {'median':>12} {'best':>12} {'throughput':>20}")
        print("-" * 80)
        gc.disable()
        try:
            for benchmark in BENCHMARKS:
                median, best, iterations = _measure(
                    benchmark,
                    minimum_seconds=arguments.min_time,
                    repeat=arguments.repeat,
                )
                print(
                    f"{benchmark.name:32} "
                    f"{median * 1_000:10.3f} ms "
                    f"{best * 1_000:10.3f} ms "
                    f"{_rate(benchmark.units, median, benchmark.unit_name):>20} "
                    f"({iterations} iterations)"
                )
        finally:
            gc.enable()
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
