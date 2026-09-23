"""Reproducible baselines for Iceberg tables, alone and beside PyIceberg.

Run after ``maturin develop --release`` with::

    python benchmarks/media/iceberg.py --min-time 0.2 --repeat 5

The first rows time ``Catalog.append`` against a name nothing has written yet:
one call resolves the dotted name against the warehouse, creates the table
from the reader's own schema, and commits the rows - data files, one
manifest, one manifest list, and one metadata document. Each call targets a
fresh table so every sample pays the same cost, rather than a snapshot history
that grows as the benchmark runs and drags the later samples down.

The rows after them put the same work beside PyIceberg
(``pip install "pyiceberg[pyarrow,sql-sqlite]==0.11.1"``) on one local
warehouse: an append of 1,048,576 six-column rows into a fresh table, once
unpartitioned and once partitioned by ``symbol``, then opening the table and
four scans of it to Arrow - everything, one partition of eight, a filter on a
non-partition column, and two of six columns. Both readers read the table
PyIceberg wrote, so they decode the same files; PyIceberg opens it from its
metadata location, this crate from the table folder. What both read is
compared before anything is timed. The ratio column is PyIceberg's median
over this crate's, so above one is in this crate's favor.

Local writes differ in one thing a timer cannot see: every file this crate
publishes on local storage is flushed to the device before the next one names
it, so a crash cannot leave a table referring to a file the disk never
received, while PyIceberg leaves the write-back to the operating system.

Without PyIceberg the second part reports ``SKIPPED`` and names what is
missing, rather than passing quietly.
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
import time
import timeit
from collections.abc import Callable
from typing import Any

import numpy as np
import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import Catalog, Table, assign_field_ids

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


# The rows an Arrow holder carries, as the plain mappings the widened write
# also takes: the delta between the two measurements is the row conversion.
ROWS = TABLE.to_pylist()

# The table both libraries write and read in the comparison.
SYMBOLS = np.array(["AAPL", "MSFT", "GOOG", "AMZN", "NVDA", "META", "TSLA", "BP"])
COMPARED_ROWS = 1 << 20


def _compared_table() -> pa.Table:
    """Six columns: a key, a UTC instant, two codes, a price and a size."""
    rng = np.random.default_rng(7)
    ids = np.arange(COMPARED_ROWS, dtype=np.int64)
    return pa.table(
        {
            "id": ids,
            "ts": pa.array((1_700_000_000_000_000 + ids * 1_000).astype("datetime64[us]")).cast(
                pa.timestamp("us", tz="UTC")
            ),
            "symbol": SYMBOLS[ids % len(SYMBOLS)],
            "price": rng.random(COMPARED_ROWS) * 1_000.0,
            "qty": rng.integers(1, 10_000, COMPARED_ROWS, dtype=np.int64),
            "venue": np.array(["XNAS", "XNYS", "ARCX"])[ids % 3],
        },
        schema=pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("ts", pa.timestamp("us", tz="UTC"), nullable=False),
                pa.field("symbol", pa.string(), nullable=False),
                pa.field("price", pa.float64(), nullable=False),
                pa.field("qty", pa.int64(), nullable=False),
                pa.field("venue", pa.string(), nullable=False),
            ]
        ),
    )


def _append_fresh_table() -> object:
    """Create one table from the rows' own schema and commit one append."""
    return CATALOG.append(next(_NAMES), TABLE).version


def _append_fresh_table_rows() -> object:
    """The same commit from plain mappings rather than an Arrow holder."""
    return CATALOG.append(next(_NAMES), ROWS).version


def _measure(
    operation: Callable[[], object], *, minimum_seconds: float, repeat: int
) -> tuple[float, float, int]:
    operation()
    number = 1
    while number < 4_096:
        if timeit.timeit(operation, number=number) >= minimum_seconds:
            break
        number *= 2
    gc.collect()
    samples = timeit.repeat(operation, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def _median(
    operation: Callable[[], object], repeat: int, setup: Callable[[], None] | None = None
) -> float:
    """The median of ``repeat`` timed calls, each after its own untimed setup."""
    laps = []
    for _ in range(repeat + 1):
        if setup is not None:
            setup()
        gc.collect()
        started = time.perf_counter()
        operation()
        laps.append(time.perf_counter() - started)
    return statistics.median(laps[1:])


def _against_pyiceberg(repeat: int) -> None:
    """Time appends, opens and scans beside PyIceberg on one warehouse."""
    try:
        from pyiceberg import __version__ as pyiceberg_version
        from pyiceberg.catalog.sql import SqlCatalog
        from pyiceberg.expressions import EqualTo, GreaterThan
        from pyiceberg.table import StaticTable
    except ImportError as error:
        print(f"against PyIceberg: SKIPPED (pyiceberg is not installed: {error})")
        return

    data = _compared_table()
    numbered = assign_field_ids(data.schema)
    root = ROOT / "compared"
    catalog = SqlCatalog(
        "bench", uri=f"sqlite:///{root}.db", warehouse=f"file://{root}/pyiceberg"
    )
    catalog.create_namespace("bench")
    counter = itertools.count()
    held: dict[str, Any] = {}

    def theirs(partitioned: bool) -> Callable[[], None]:
        def setup() -> None:
            table = catalog.create_table(f"bench.t{next(counter)}", schema=data.schema)
            if partitioned:
                with table.update_spec() as update:
                    update.add_identity("symbol")
            held["theirs"] = table

        return setup

    def ours(partitioned: bool) -> Callable[[], None]:
        def setup() -> None:
            folder = IOBase(root / "yggdryl" / f"t{next(counter)}")
            held["ours"] = Table.create(folder, numbered, ["symbol"] if partitioned else None)

        return setup

    print()
    print(
        f"against PyIceberg {pyiceberg_version}: "
        f"{COMPARED_ROWS:,} rows, {len(data.schema)} columns"
    )
    print(f"{'operation':36} {'yggdryl':>12} {'PyIceberg':>12} {'ratio':>7}")
    print("-" * 70)

    def report(name: str, mine: float, other: float) -> None:
        print(f"{name:36} {mine * 1e3:9.2f} ms {other * 1e3:9.2f} ms {other / mine:7.2f}", flush=True)

    for partitioned in (False, True):
        label = "append, partitioned by symbol" if partitioned else "append, unpartitioned"
        mine = _median(lambda: held["ours"].append(data), repeat, ours(partitioned))
        other = _median(lambda: held["theirs"].append(data), repeat, theirs(partitioned))
        report(label, mine, other)

    for partitioned in (False, True):
        theirs(partitioned)()
        written = held["theirs"]
        written.append(data)
        written = catalog.load_table(written.name())
        location = pathlib.Path(written.location().removeprefix("file://"))
        metadata = written.metadata_location
        suffix = " (8 partitions)" if partitioned else ""
        mine_table = Table.open(IOBase(location))
        their_table = StaticTable.from_metadata(metadata)
        projection = pa.schema([data.schema.field("id"), data.schema.field("price")])
        cases = [
            ("open", lambda: Table.open(IOBase(location)), lambda: StaticTable.from_metadata(metadata)),
            ("scan everything", lambda: mine_table.scan().read_all(), lambda: their_table.scan().to_arrow()),
            (
                "scan symbol = 'AAPL'",
                lambda: mine_table.scan_matching("symbol = 'AAPL'").read_all(),
                lambda: their_table.scan(row_filter=EqualTo("symbol", "AAPL")).to_arrow(),
            ),
            (
                "scan price > 900",
                lambda: mine_table.scan_matching("price > 900.0").read_all(),
                lambda: their_table.scan(row_filter=GreaterThan("price", 900.0)).to_arrow(),
            ),
            (
                "scan id, price",
                lambda: mine_table.scan(projection).read_all(),
                lambda: their_table.scan(selected_fields=("id", "price")).to_arrow(),
            ),
        ]
        for name, mine_operation, their_operation in cases:
            if name != "open":
                read, expected = mine_operation(), their_operation()
                if (read.num_rows, read.num_columns) != (expected.num_rows, expected.num_columns):
                    raise SystemExit(f"{name}: the two readers disagree on what the table holds")
            report(name + suffix, _median(mine_operation, repeat), _median(their_operation, repeat))


def main() -> None:
    """Time create-on-first-write appends, then the comparison with PyIceberg."""
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
            measured = [
                ("catalog append arrow table", _append_fresh_table),
                ("catalog append plain rows", _append_fresh_table_rows),
            ]
            for name, operation in measured:
                median, best, iterations = _measure(
                    operation,
                    minimum_seconds=arguments.min_time,
                    repeat=arguments.repeat,
                )
                rate = f"{ROW_COUNT / median:,.0f} rows/s"
                print(
                    f"{name:32} "
                    f"{median * 1_000:10.3f} ms "
                    f"{best * 1_000:10.3f} ms "
                    f"{rate:>20} "
                    f"({iterations} iterations)"
                )
        finally:
            gc.enable()
        _against_pyiceberg(arguments.repeat)
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
