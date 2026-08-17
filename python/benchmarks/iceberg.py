"""Reproducible baselines for the Iceberg catalog boundary.

Run after ``maturin develop --features "yggdryl/parquet,yggdryl/iceberg"``
with::

    python benchmarks/iceberg.py --min-time 0.2 --repeat 5

The operation measured is ``Catalog.append`` against a name nothing has
written yet: one call resolves the dotted name against the warehouse, creates
the table from the reader's own schema, and commits the rows - data files, one
manifest, one manifest list, and one metadata document. Each call targets a
fresh table so every sample pays the same cost, rather than a snapshot history
that grows as the benchmark runs and drags the later samples down.
"""

from __future__ import annotations

import argparse
import gc
import itertools
import pathlib
import platform
import shutil
import statistics
import tempfile
import timeit

import pyarrow as pa

from yggdryl.iceberg import Catalog

ROW_COUNT = 65_536
BATCH_SIZE = 8_192

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ]
)
BATCHES = tuple(
    pa.record_batch(
        {
            "id": list(range(start, start + BATCH_SIZE)),
            "symbol": ["AAPL"] * BATCH_SIZE,
            "price": [float(start)] * BATCH_SIZE,
        },
        schema=SCHEMA,
    )
    for start in range(0, ROW_COUNT, BATCH_SIZE)
)
TABLE = pa.Table.from_batches(BATCHES, schema=SCHEMA)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-bench-"))
CATALOG = Catalog(ROOT / "warehouse")
_NAMES = (f"bench.t{index}" for index in itertools.count())


def _append_fresh_table() -> object:
    """Create one table from the rows' own schema and commit one append."""
    return CATALOG.append(next(_NAMES), TABLE).version


def _measure(*, minimum_seconds: float, repeat: int) -> tuple[float, float, int]:
    _append_fresh_table()
    number = 1
    while number < 4_096:
        if timeit.timeit(_append_fresh_table, number=number) >= minimum_seconds:
            break
        number *= 2
    gc.collect()
    samples = timeit.repeat(_append_fresh_table, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def main() -> None:
    """Time create-on-first-write appends against a fresh warehouse."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    try:
        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{ROW_COUNT:,} rows, {len(SCHEMA)} columns, {len(BATCHES)} batches "
            "per append"
        )
        print(f"{'benchmark':32} {'median':>12} {'best':>12} {'throughput':>20}")
        print("-" * 80)
        gc.disable()
        try:
            median, best, iterations = _measure(
                minimum_seconds=arguments.min_time, repeat=arguments.repeat
            )
        finally:
            gc.enable()
        rate = f"{ROW_COUNT / median:,.0f} rows/s"
        print(
            f"{'catalog append fresh table':32} "
            f"{median * 1_000:10.3f} ms "
            f"{best * 1_000:10.3f} ms "
            f"{rate:>20} "
            f"({iterations} iterations)"
        )
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
