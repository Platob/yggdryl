"""Measure plain-text records through the generic media boundary.

Run from ``python/`` against a release wheel::

    .venv/Scripts/python benchmarks/text.py --min-time 0.2 --repeat 7
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
from dataclasses import dataclass
from datetime import datetime
from typing import Callable

import pyarrow as pa

from yggdryl import IOBase, TextOptions

ROWS = 50_000
ROWHEADER = r"^(?P<stamp>\S+) \[(?P<level>[A-Z]+)\] id=(?P<id>\d+)"
COMPILED = re.compile(ROWHEADER)
ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-text-"))
PLAIN = ROOT / "events.log"
CODED = ROOT / "events.log.gz"


def corpus() -> str:
    return "".join(
        f"2026-08-14T00:05:{index % 60:02d} "
        f"[{'WARN' if index % 3 == 0 else 'INFO'}] id={index} event {index}\n"
        for index in range(ROWS)
    )


def options() -> TextOptions:
    value = TextOptions()
    value.rowheader = ROWHEADER
    value.lstrip = r"^\s+"
    value.rstrip = r"\s+$"
    return value


TEXT_OPTIONS = options()


def native(target: pathlib.Path) -> int:
    handle = IOBase(target).into_text(TEXT_OPTIONS)
    return sum(
        batch.num_rows
        for batch in handle.read_arrow_reader()
    )


def baseline(target: pathlib.Path) -> int:
    encoded = target.read_bytes()
    text = gzip.decompress(encoded).decode() if target.suffix == ".gz" else encoded.decode()
    columns: dict[str, list[object]] = {
        "url": [],
        "rownum": [],
        "body": [],
        "stamp": [],
        "level": [],
        "id": [],
    }
    for rownum, line in enumerate(text.splitlines(), 1):
        found = COMPILED.search(line)
        assert found is not None
        columns["url"].append(target.as_uri())
        columns["rownum"].append(rownum)
        columns["body"].append((line[: found.start()] + line[found.end() :]).strip().encode())
        columns["stamp"].append(datetime.fromisoformat(found.group("stamp")))
        columns["level"].append(found.group("level"))
        columns["id"].append(int(found.group("id")))
    return pa.table(columns).num_rows


@dataclass(frozen=True)
class Benchmark:
    name: str
    operation: Callable[[], int]


def measure(
    benchmark: Benchmark, minimum_seconds: float, repeat: int
) -> tuple[float, float, int]:
    benchmark.operation()
    number = 1
    while number < 4_096 and timeit.timeit(benchmark.operation, number=number) < minimum_seconds:
        number *= 2
    gc.collect()
    samples = timeit.repeat(benchmark.operation, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=7)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    try:
        text = corpus()
        PLAIN.write_text(text, encoding="utf-8")
        CODED.write_bytes(gzip.compress(text.encode()))
        benchmarks = (
            Benchmark("read_arrow_reader plain", lambda: native(PLAIN)),
            Benchmark("read_arrow_reader gzip", lambda: native(CODED)),
            Benchmark("python re + pyarrow plain", lambda: baseline(PLAIN)),
            Benchmark("python re + pyarrow gzip", lambda: baseline(CODED)),
        )
        assert all(benchmark.operation() == ROWS for benchmark in benchmarks)

        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{ROWS:,} rows, {len(text):,} decoded bytes"
        )
        for benchmark in benchmarks:
            median, best, iterations = measure(
                benchmark, arguments.min_time, arguments.repeat
            )
            print(
                f"{benchmark.name:28} {median * 1_000:10.3f} ms median "
                f"{best * 1_000:10.3f} ms best "
                f"{ROWS / median:12,.0f} rows/s ({iterations} iterations)"
            )
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
