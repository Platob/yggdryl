"""Parquet reads and writes beside PyArrow, on identical files.

Run after ``maturin develop --release`` with::

    python benchmarks/media/parquet.py --repeat 7

Every read case writes one file with PyArrow and then reads that same file
both ways, so the two readers decode identical bytes. Four readers are timed
per case: ``pyarrow.parquet.read_table`` and ``ParquetFile.iter_batches`` on
the PyArrow side, and ``IOBase.read_arrow_reader`` drained whole
(``read_all``) and streamed batch by batch on this side. The ratio columns
are PyArrow's time over this crate's, so above one is in this crate's favor:
whole table against whole table, stream against stream.

The filtered cases read one 4M-row file of 32 row groups through a filter:
``read_table(filters=...)`` on one side, a ``filter`` string on the other.
On a sorted key both skip the row groups the footer statistics rule out;
on the other columns every row group is read and its rows tested.

The write cases time ``pyarrow.parquet.write_table`` against
``IOBase.overwrite_arrow_table`` for the same table, Zstandard level 1 - both
libraries' default - each into a file of its own.

The shapes cover the layouts a reader meets: one large row group and several,
Zstandard, Snappy and uncompressed pages, a two-column projection, a
string-heavy table, and a small file where per-file overhead dominates.
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import platform
import shutil
import tempfile
import time
from collections.abc import Callable, Iterable

import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

from yggdryl import IOBase

SYMBOLS = np.array(["AAPL", "MSFT", "GOOG", "AMZN", "NVDA", "META", "TSLA", "BP"])

# One filter string each, beside the PyArrow filter that keeps the same rows.
FILTERS = [
    ("id < 100,000, 1 row group of 32", "id < 100000", [("id", "<", 100_000)]),
    (
        "id in 2.0M..2.1M, 1 row group of 32",
        "id between 2000000 and 2100000",
        [("id", ">=", 2_000_000), ("id", "<=", 2_100_000)],
    ),
    ("price > 990, 1% of rows", "price > 990.0", [("price", ">", 990.0)]),
    ("symbol = 'AAPL', 12.5% of rows", "symbol = 'AAPL'", [("symbol", "=", "AAPL")]),
]
VENUES = np.array(["XNAS", "XNYS", "ARCX"])


def trades(rows: int) -> pa.Table:
    """Six mixed columns: a key, an instant, two codes, a price, a size."""
    rng = np.random.default_rng(7)
    ids = np.arange(rows, dtype=np.int64)
    return pa.table(
        {
            "id": ids,
            "ts": pa.array((1_700_000_000_000_000 + ids * 1_000).astype("datetime64[us]")),
            "symbol": SYMBOLS[ids % len(SYMBOLS)],
            "price": rng.random(rows) * 1_000.0,
            "qty": rng.integers(1, 10_000, rows, dtype=np.int64),
            "venue": VENUES[ids % len(VENUES)],
        }
    )


def notes(rows: int) -> pa.Table:
    """A string-heavy table: free text, a code, and a flag."""
    rng = np.random.default_rng(3)
    words = np.array([f"order-{index:08x}-{'x' * (index % 40)}" for index in range(50_000)])
    clients = np.array([f"client-{index}" for index in range(1_000)])
    return pa.table(
        {
            "id": np.arange(rows, dtype=np.int64),
            "note": words[rng.integers(0, len(words), rows)],
            "client": clients[rng.integers(0, len(clients), rows)],
            "flag": rng.random(rows) > 0.5,
        }
    )


def _best(operation: Callable[[], object], repeat: int) -> float:
    """The fastest of ``repeat`` timed calls, after one warm-up call."""
    operation()
    laps = []
    for _ in range(repeat):
        gc.collect()
        started = time.perf_counter()
        operation()
        laps.append(time.perf_counter() - started)
    return min(laps)


def _drain(batches: Iterable[pa.RecordBatch]) -> int:
    return sum(batch.num_rows for batch in batches)


def main() -> None:
    """Time each read and write case both ways and print one row per case."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=7)
    parser.add_argument("--filter", default="", help="run only cases naming this text")
    arguments = parser.parse_args()
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    million, four_million = 1 << 20, 1 << 22
    table = trades(million)
    reads = [
        ("trades 1M rows, 1 row group, zstd", table, {"compression": "zstd", "row_group_size": million}, None),
        ("trades 1M rows, 8 row groups, zstd", table, {"compression": "zstd", "row_group_size": million // 8}, None),
        ("trades 1M rows, 1 row group, snappy", table, {"compression": "snappy", "row_group_size": million}, None),
        ("trades 1M rows, uncompressed", table, {"compression": "none", "row_group_size": million}, None),
        ("trades 1M rows, 2 of 6 columns", table, {"compression": "zstd", "row_group_size": million}, ["id", "price"]),
        ("trades 4M rows, 4 row groups, zstd", trades(four_million), {"compression": "zstd", "row_group_size": million}, None),
        ("notes 1M rows, strings, zstd", notes(million), {"compression": "zstd", "row_group_size": million}, None),
        ("trades 64K rows, zstd", trades(1 << 16), {"compression": "zstd"}, None),
    ]
    writes = [
        ("trades 1M rows", table),
        ("trades 4M rows", reads[5][1]),
        ("notes 1M rows, strings", reads[6][1]),
        ("trades 64K rows", reads[7][1]),
    ]

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-parquet-bench-"))
    try:
        print(f"Python {platform.python_version()}, PyArrow {pa.__version__}")
        print()
        print(
            f"{'read':38} {'read_table':>11} {'iter_batches':>13} "
            f"{'read_all':>10} {'stream':>10} {'x table':>8} {'x stream':>9}"
        )
        for index, (name, source, options, columns) in enumerate(reads):
            if arguments.filter not in name:
                continue
            path = root / f"read-{index}.parquet"
            pq.write_table(source, path, **options)
            select = {} if columns is None else {"select": ", ".join(columns)}
            expected = pq.read_table(path, columns=columns)
            actual = IOBase(path).read_arrow_reader(**select).read_all()
            if (actual.num_rows, actual.num_columns) != (expected.num_rows, expected.num_columns):
                raise SystemExit(f"{name}: the two readers disagree on what the file holds")
            table_time = _best(lambda: pq.read_table(path, columns=columns), arguments.repeat)
            iter_time = _best(
                lambda: _drain(pq.ParquetFile(path).iter_batches(batch_size=65_536, columns=columns)),
                arguments.repeat,
            )
            all_time = _best(lambda: IOBase(path).read_arrow_reader(**select).read_all(), arguments.repeat)
            stream_time = _best(lambda: _drain(IOBase(path).read_arrow_reader(**select)), arguments.repeat)
            print(
                f"{name:38} {table_time * 1e3:8.2f} ms {iter_time * 1e3:10.2f} ms "
                f"{all_time * 1e3:7.2f} ms {stream_time * 1e3:7.2f} ms "
                f"{table_time / all_time:8.2f} {iter_time / stream_time:9.2f}",
                flush=True,
            )
        print()
        print(f"{'filtered read':38} {'read_table':>11} {'read_all':>13} {'x':>8}")
        path = root / "filtered.parquet"
        pq.write_table(reads[5][1], path, compression="zstd", row_group_size=1 << 17)
        for name, text, filters in FILTERS:
            if arguments.filter not in name:
                continue
            expected = pq.read_table(path, filters=filters)
            actual = IOBase(path).read_arrow_reader(filter=text).read_all()
            if actual.num_rows != expected.num_rows:
                raise SystemExit(f"{name}: the two readers disagree on the rows the filter keeps")
            their_time = _best(lambda: pq.read_table(path, filters=filters), arguments.repeat)
            our_time = _best(
                lambda: IOBase(path).read_arrow_reader(filter=text).read_all(), arguments.repeat
            )
            print(
                f"{name:38} {their_time * 1e3:8.2f} ms {our_time * 1e3:10.2f} ms "
                f"{their_time / our_time:8.2f}",
                flush=True,
            )
        print()
        print(f"{'write':38} {'write_table':>11} {'overwrite':>13} {'x':>8}")
        for index, (name, source) in enumerate(writes):
            if arguments.filter not in name:
                continue
            theirs, ours = root / f"write-{index}-pyarrow.parquet", root / f"write-{index}.parquet"
            their_time = _best(lambda: pq.write_table(source, theirs, compression="zstd"), arguments.repeat)
            our_time = _best(lambda: IOBase(ours).overwrite_arrow_table(source), arguments.repeat)
            if pq.read_table(ours).num_rows != source.num_rows:
                raise SystemExit(f"{name}: the written file does not hold the rows")
            print(
                f"{name:38} {their_time * 1e3:8.2f} ms {our_time * 1e3:10.2f} ms "
                f"{their_time / our_time:8.2f}",
                flush=True,
            )
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
