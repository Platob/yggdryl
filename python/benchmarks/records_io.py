"""Reproducible baselines for the record boundary: readers in, readers out.

Run after ``maturin develop`` with::

    python benchmarks/records_io.py --min-time 0.2 --repeat 7

The boundary measured here is the Arrow C Stream interface: a PyArrow reader
becomes a core batch reader on the way in and a core batch reader becomes a
PyArrow reader on the way out. Fixtures are written before timing, and the
column-pushdown pair reports the bytes each read materializes so "moves less
data" is a measured number rather than an inference from elapsed time.

The ``generic``, ``pandas``, and ``polars`` rows measure the wider boundary the
inferring entry points cross. Comparing ``generic write table`` against ``ipc
write reader`` is what says how much the inference itself costs, and the row
and frame cases say what it costs to build Arrow out of something that is not
Arrow yet. A frame library that is not installed is left out of the table
rather than reported as a zero.
"""

from __future__ import annotations

import argparse
import gc
import importlib
import pathlib
import platform
import shutil
import statistics
import tempfile
import timeit
from dataclasses import dataclass
from typing import Callable

import pyarrow as pa

from yggdryl import IOBase

ROW_COUNT = 65_536
BATCH_SIZE = 8_192

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ]
)
WANTED = pa.schema([pa.field("id", pa.int64(), nullable=False)])

BATCHES = tuple(
    pa.record_batch(
        {
            "id": list(range(start, start + BATCH_SIZE)),
            "symbol": ["AAPL"] * BATCH_SIZE,
            "venue": ["XNAS"] * BATCH_SIZE,
            "price": [float(start)] * BATCH_SIZE,
        },
        schema=SCHEMA,
    )
    for start in range(0, ROW_COUNT, BATCH_SIZE)
)
TABLE = pa.Table.from_batches(BATCHES, schema=SCHEMA)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-bench-"))
STREAM = IOBase(ROOT / "trades.arrows")
FILE = IOBase(ROOT / "trades.parquet")
SINK_STREAM = IOBase(ROOT / "sink.arrows")
SINK_FILE = IOBase(ROOT / "sink.parquet")


@dataclass(frozen=True)
class Benchmark:
    """One measured operation and the unit its throughput is reported in."""

    name: str
    operation: Callable[[], object]
    units: int
    unit_name: str


def _materialized(handle: IOBase, field: object | None) -> int:
    """Read every batch and report the bytes the read actually built."""
    options = handle.record_options()
    if field is not None:
        options.schema = field
    return sum(batch.nbytes for batch in handle.read_arrow_batch_reader(options=options))


def _write_stream() -> object:
    SINK_STREAM.write_arrow_batch_reader(TABLE)
    return SINK_STREAM.size


def _write_file() -> object:
    SINK_FILE.write_arrow_batch_reader(TABLE)
    return SINK_FILE.size


def _read_stream_whole() -> object:
    return _materialized(STREAM, None)


def _read_stream_subset() -> object:
    return _materialized(STREAM, WANTED)


def _read_file_whole() -> object:
    return _materialized(FILE, None)


def _read_file_subset() -> object:
    return _materialized(FILE, WANTED)


def _read_stream_table() -> object:
    return STREAM.read_arrow_batch_reader().read_all().num_rows


def _pyarrow_ipc_baseline() -> object:
    # The same work through PyArrow's own writer, to the same kind of
    # destination, so the two numbers are comparable.
    with pa.OSFile(str(ROOT / "baseline.arrows"), "wb") as sink:
        with pa.ipc.new_stream(sink, SCHEMA) as writer:
            for batch in BATCHES:
                writer.write_batch(batch)
    return (ROOT / "baseline.arrows").stat().st_size


# The generic entry points cross a wider boundary than the reader pair above:
# each one infers what it was handed before the same three core methods run, so
# these numbers say what that inference costs on top of the write itself.
def _write_generic_table() -> object:
    SINK_STREAM.write_arrow(TABLE)
    return SINK_STREAM.size


def _write_generic_generator() -> object:
    # A generator of tables is chained rather than collected, so this measures
    # the per-item pull as well as the write.
    SINK_STREAM.write_arrow(batch for batch in BATCHES)
    return SINK_STREAM.size


def _write_generic_rows() -> object:
    SINK_STREAM.write_arrow(iter(ROW_MAPPINGS))
    return SINK_STREAM.size


def _optional_frame(package: str) -> object | None:
    """Build one frame of the fixture rows, or report the library's absence."""
    try:
        library = importlib.import_module(package)
    except ImportError:
        return None
    if package == "pandas":
        return TABLE.to_pandas()
    return library.from_arrow(TABLE)


PANDAS_FRAME = _optional_frame("pandas")
POLARS_FRAME = _optional_frame("polars")
ROW_MAPPINGS = TABLE.to_pylist()


def _write_pandas_frame() -> object:
    SINK_FILE.write_pandas_frame(PANDAS_FRAME)
    return SINK_FILE.size


def _read_pandas_frame() -> object:
    return len(FILE.read_pandas_frame())


def _write_polars_frame() -> object:
    SINK_FILE.write_polars_frame(POLARS_FRAME)
    return SINK_FILE.size


def _read_polars_frame() -> object:
    return FILE.read_polars_frame().height


BENCHMARKS = tuple(
    benchmark
    for benchmark in (
        Benchmark("ipc write reader", _write_stream, ROW_COUNT, "row"),
        Benchmark("ipc read whole", _read_stream_whole, ROW_COUNT, "row"),
        Benchmark("ipc read subset", _read_stream_subset, ROW_COUNT, "row"),
        Benchmark("ipc read to table", _read_stream_table, ROW_COUNT, "row"),
        Benchmark("parquet write reader", _write_file, ROW_COUNT, "row"),
        Benchmark("parquet read whole", _read_file_whole, ROW_COUNT, "row"),
        Benchmark("parquet read subset", _read_file_subset, ROW_COUNT, "row"),
        Benchmark("PyArrow IPC write baseline", _pyarrow_ipc_baseline, ROW_COUNT, "row"),
        Benchmark("generic write table", _write_generic_table, ROW_COUNT, "row"),
        Benchmark("generic write generator", _write_generic_generator, ROW_COUNT, "row"),
        Benchmark("generic write mappings", _write_generic_rows, ROW_COUNT, "row"),
        Benchmark("pandas write frame", _write_pandas_frame, ROW_COUNT, "row")
        if PANDAS_FRAME is not None
        else None,
        Benchmark("pandas read frame", _read_pandas_frame, ROW_COUNT, "row")
        if PANDAS_FRAME is not None
        else None,
        Benchmark("polars write frame", _write_polars_frame, ROW_COUNT, "row")
        if POLARS_FRAME is not None
        else None,
        Benchmark("polars read frame", _read_polars_frame, ROW_COUNT, "row")
        if POLARS_FRAME is not None
        else None,
    )
    # A frame library nobody installed is a benchmark that is not run, never a
    # zero that reads as a measurement.
    if benchmark is not None
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
    """Write the fixtures, then time every operation over them."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=7)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    try:
        STREAM.write_arrow_batch_reader(TABLE)
        FILE.write_arrow_batch_reader(TABLE)

        whole_stream = _materialized(STREAM, None)
        subset_stream = _materialized(STREAM, WANTED)
        whole_file = _materialized(FILE, None)
        subset_file = _materialized(FILE, WANTED)
        # A pushdown that stopped pushing down would still be correct and would
        # still be fast enough to look fine, so the bytes are asserted first.
        assert subset_stream < whole_stream
        assert subset_file < whole_file

        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{ROW_COUNT:,} rows, {len(SCHEMA)} columns, {len(BATCHES)} batches"
        )
        print(
            f"materialized: ipc {whole_stream:,} -> {subset_stream:,} bytes, "
            f"parquet {whole_file:,} -> {subset_file:,} bytes"
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
        # The handles memory-map their files, so they are released before the
        # directory they live in is removed.
        for handle in (STREAM, FILE, SINK_STREAM, SINK_FILE):
            handle.close()
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
