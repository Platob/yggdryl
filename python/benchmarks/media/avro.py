"""Avro reads and writes beside polars and fastavro, on identical files.

Run after ``maturin develop --release`` with::

    python benchmarks/media/avro.py --repeat 5

Every read case writes one container with fastavro - in blocks of about
64,000 bytes, the Java writer's default - and then reads that same file three
ways, so the readers decode identical bytes: ``polars.read_avro``
(columnar, into Arrow memory), ``fastavro.reader`` (one Python mapping per
row, for scale), and ``IOBase.read_arrow_reader(...).read_all()`` on this
side. The ratio columns are the other reader's time over this crate's, so
above one is in this crate's favor. Before anything is timed, the key and
price columns polars reads are compared with this crate's, value for value.
polars cannot read Zstandard blocks - it refuses them or misreads them - so
that row times fastavro alone.

The write cases time ``polars.DataFrame.write_avro`` and ``fastavro.writer``
against ``IOBase.overwrite_arrow_table`` for the same table and block codec,
each into a file of its own; fastavro is handed the rows it needs as Python
mappings, prepared before the timer starts.

Neither package is a dependency of the crate - they are checking tools of this
script only (``pip install polars fastavro``); without them the script names
what is missing and stops.
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import platform
import shutil
import tempfile
import time
from collections.abc import Callable

import numpy as np
import pyarrow as pa

from yggdryl import IOBase

SYMBOLS = np.array(["AAPL", "MSFT", "GOOG", "AMZN", "NVDA", "META", "TSLA", "BP"])
VENUES = np.array(["XNAS", "XNYS", "ARCX"])

# Bytes per block the read fixtures are written with: the Java writer's
# default, and so the block size most containers in the wild carry.
SYNC_INTERVAL = 64_000

SCHEMA = {
    "type": "record",
    "name": "trade",
    "fields": [
        {"name": "id", "type": "long"},
        {"name": "ts", "type": {"type": "long", "logicalType": "timestamp-micros"}},
        {"name": "symbol", "type": "string"},
        {"name": "price", "type": "double"},
        {"name": "qty", "type": "long"},
        {"name": "venue", "type": "string"},
    ],
}


def trades(rows: int) -> pa.Table:
    """Six mixed columns: a key, an instant, two codes, a price, a size."""
    rng = np.random.default_rng(7)
    ids = np.arange(rows, dtype=np.int64)
    return pa.table(
        {
            "id": ids,
            "ts": pa.array((1_700_000_000_000_000 + ids * 1_000).astype("datetime64[us]")).cast(
                pa.timestamp("us", tz="UTC")
            ),
            "symbol": SYMBOLS[ids % len(SYMBOLS)],
            "price": rng.random(rows) * 1_000.0,
            "qty": rng.integers(1, 10_000, rows, dtype=np.int64),
            "venue": VENUES[ids % len(VENUES)],
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


def _records(table: pa.Table) -> list[dict[str, object]]:
    """The rows as fastavro takes them: instants as integer microseconds."""
    plain = table.set_column(1, "ts", table.column("ts").cast(pa.int64()))
    return plain.to_pylist()


def main() -> None:
    """Time each read and write case three ways and print one row per case."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--filter", default="", help="run only cases naming this text")
    parser.add_argument("--no-fastavro", action="store_true", help="skip the per-row fastavro reader")
    arguments = parser.parse_args()
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")
    try:
        import fastavro
        import polars as pl
    except ImportError as error:
        raise SystemExit(f"avro benchmark: SKIPPED ({error}); pip install polars fastavro")

    million = 1 << 20
    table = trades(million)
    records = _records(table)
    small = trades(1 << 16)
    small_records = _records(small)
    reads = [
        ("trades 1M rows, null", records, "null", None),
        ("trades 1M rows, deflate", records, "deflate", None),
        ("trades 1M rows, snappy", records, "snappy", None),
        ("trades 1M rows, zstandard", records, "zstandard", None),
        ("trades 1M rows, 2 of 6 columns, snappy", records, "snappy", ["id", "price"]),
        ("trades 64K rows, deflate", small_records, "deflate", None),
    ]
    writes = [
        ("trades 1M rows, null", table, records, "null"),
        ("trades 1M rows, deflate", table, records, "deflate"),
        ("trades 1M rows, snappy", table, records, "snappy"),
        ("trades 64K rows, deflate", small, small_records, "deflate"),
    ]
    polars_codecs = {"null": "uncompressed", "deflate": "deflate", "snappy": "snappy"}

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-avro-bench-"))
    parsed = fastavro.parse_schema(SCHEMA)
    try:
        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}, "
            f"polars {pl.__version__}, fastavro {fastavro.__version__}"
        )
        print()
        print(
            f"{'read':40} {'polars':>10} {'fastavro':>11} {'yggdryl':>10} "
            f"{'x polars':>9} {'x fastavro':>11}"
        )
        for index, (name, rows, codec, columns) in enumerate(reads):
            if arguments.filter not in name:
                continue
            path = root / f"read-{index}.avro"
            with path.open("wb") as sink:
                fastavro.writer(sink, parsed, rows, codec=codec, sync_interval=SYNC_INTERVAL)
            select = {} if columns is None else {"select": ", ".join(columns)}
            actual = IOBase(path).read_arrow_reader(**select).read_all()
            if codec == "zstandard":
                polars_time = float("nan")
            else:
                expected = pl.read_avro(path, columns=columns).to_arrow()
                if (actual.num_rows, actual.num_columns) != expected.shape or any(
                    not actual.column(column).equals(expected.column(column))
                    for column in ("id", "price")
                ):
                    raise SystemExit(f"{name}: the readers disagree on what the file holds")
                polars_time = _best(lambda: pl.read_avro(path, columns=columns), arguments.repeat)
            ours = _best(lambda: IOBase(path).read_arrow_reader(**select).read_all(), arguments.repeat)
            if arguments.no_fastavro:
                fast_time = float("nan")
            else:

                def drain() -> int:
                    with path.open("rb") as source:
                        return sum(1 for _ in fastavro.reader(source))

                fast_time = _best(drain, 1)
            print(
                f"{name:40} {polars_time * 1e3:7.2f} ms {fast_time * 1e3:8.1f} ms "
                f"{ours * 1e3:7.2f} ms {polars_time / ours:9.2f} {fast_time / ours:11.2f}",
                flush=True,
            )
        print()
        print(
            f"{'write':40} {'polars':>10} {'fastavro':>11} {'yggdryl':>10} "
            f"{'x polars':>9} {'x fastavro':>11}"
        )
        for index, (name, source, rows, codec) in enumerate(writes):
            if arguments.filter not in name:
                continue
            # polars writes no zoned instant, so its frame carries the same
            # microseconds without the zone - the same bytes on the wire.
            frame = pl.from_arrow(source).with_columns(pl.col("ts").dt.replace_time_zone(None))
            theirs = root / f"write-{index}-polars.avro"
            fast = root / f"write-{index}-fastavro.avro"
            ours_path = root / f"write-{index}.avro"
            polars_time = _best(
                lambda: frame.write_avro(theirs, compression=polars_codecs[codec]), arguments.repeat
            )

            def fast_write() -> None:
                with fast.open("wb") as sink:
                    fastavro.writer(sink, parsed, rows, codec=codec)

            fast_time = float("nan") if arguments.no_fastavro else _best(fast_write, 1)
            ours = _best(
                lambda: IOBase(ours_path).overwrite_arrow_table(source, block_codec=codec),
                arguments.repeat,
            )
            if pl.read_avro(ours_path).height != source.num_rows:
                raise SystemExit(f"{name}: the written file does not hold the rows")
            print(
                f"{name:40} {polars_time * 1e3:7.2f} ms {fast_time * 1e3:8.1f} ms "
                f"{ours * 1e3:7.2f} ms {polars_time / ours:9.2f} {fast_time / ours:11.2f}",
                flush=True,
            )
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
