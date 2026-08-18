"""Time the Arrow line projection against a plain-Python parsing loop.

Run from ``python/`` after ``maturin develop --release`` (a debug build
understates the native side by an order of magnitude)::

    .venv/Scripts/python benchmarks/log_lines.py --min-time 0.2 --repeat 7

The payload is ~100k synthetic trading-log lines, timed uncompressed and
gzip-coded. The baseline is what a Python engineer would write without the
binding: ``re`` with named groups over ``str.splitlines``, timestamps through
``datetime.fromisoformat``, columns accumulated into lists and handed to
``pyarrow.table``. The baseline hashes messages with ``zlib.crc32`` - a
C-speed 32-bit checksum - while the binding pays for the stable 64-bit
FNV-1a the ``hash`` column contractually carries, so the comparison flatters
the baseline slightly and is still a fair "same job" measurement. PyArrow's
CSV reader is deliberately absent: a regex-matched log line is not a CSV row,
and a baseline doing a different job is not a baseline.

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
import timeit
import zlib
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Callable

import pyarrow as pa

from yggdryl import IOBase

LINES = 100_000

PATTERN = (
    r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*"
    r" \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]"
)

# Python's re spells named groups (?P<name>...); the pattern is otherwise the
# same expression the binding compiles.
BASELINE_PATTERN = re.compile(
    r"^(?P<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*)"
    r" \[(?P<level>[^\]]+)\] \[(?P<logger>[^\]]+)\]"
)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-log-lines-"))
PLAIN = ROOT / "bench.log"
CODED = ROOT / "bench.log.gz"

EPOCH = datetime(1970, 1, 1)


def _corpus() -> str:
    lines = []
    for index in range(LINES):
        minute, second, micro = index // 3_600 % 60, index // 60 % 60, index % 1_000_000
        level = ("ii", "ww", "ee")[index % 3]
        price = 187.0 + (index % 400) / 100
        lines.append(
            f"2024-02-01 10:{minute:02}:{second:02}.{micro:06} [{level}] [engine]"
            f" fill {100 + index % 900} SYMB-{index % 128:04} @ {price:.2f}"
            f" order={index:08}"
        )
    return "\n".join(lines) + "\n"


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
    columns: dict[str, list] = {
        name: []
        for name in ("rownum", "unix", "hash", "header", "message", "level", "logger")
    }
    rownum = 0
    for line in text.splitlines():
        matched = BASELINE_PATTERN.match(line)
        rownum += 1
        if matched is None:
            # The multi-line grouping the binding does is folded down to the
            # header row here; a preamble or continuation stays one row so
            # the two sides parse the same number of headers.
            continue
        message = line[matched.end() :].strip()
        stamp = datetime.fromisoformat(matched.group("stamp").replace("_", ""))
        columns["rownum"].append(rownum)
        columns["unix"].append(int((stamp - EPOCH).total_seconds() * 1_000_000) * 1_000)
        columns["hash"].append(zlib.crc32(message.encode()))
        columns["header"].append(matched.group(0))
        columns["message"].append(message)
        columns["level"].append(matched.group("level"))
        columns["logger"].append(matched.group("logger"))
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
            f"{LINES:,} log lines, {len(text):,} decoded bytes"
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
