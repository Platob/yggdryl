"""Reproducible baselines for the record boundary: readers in, readers out.

Run after ``maturin develop`` with::

    python benchmarks/records_io.py --min-time 0.2 --repeat 7

The boundary measured here is the Arrow C Stream interface: a PyArrow reader
becomes a core batch reader on the way in and a core batch reader becomes a
PyArrow reader on the way out. Fixtures are written before timing, and the
column-pushdown pair reports the bytes each read materializes so "moves less
data" is a measured number rather than an inference from elapsed time.
Parquet footer and projected-WKB statistics additionally measure the core
`Value` projection into one native Python record.

The intent matrix measures reader, table, record-batch, records, pandas, and
polars adapters under overwrite, append, and merge. Append and merge reset a
bounded stored table before each timed call; setup is outside the measurement.
The reader and native-row paths are also measured with an 8,192-row publication
cadence, exposing the cost of durable prefixes beside the one-publication
default. A frame library that is not installed is left out rather than reported
as zero.
"""

from __future__ import annotations

import argparse
import gc
import importlib
import pathlib
import platform
import shutil
import statistics
import struct
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
GEO_ROW_COUNT = 8_192
WKB_POINT = b"\x01\x01\x00\x00\x00" + struct.pack("<dd", 1.0, 2.0)
GEO_TABLE = pa.table(
    {"shape": pa.array([WKB_POINT] * GEO_ROW_COUNT, type=pa.binary())}
)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-bench-"))
STREAM = IOBase(ROOT / "trades.arrows")
FILE = IOBase(ROOT / "trades.parquet")
GEO_FILE = IOBase(ROOT / "shapes.parquet")
SINK_STREAM = IOBase(ROOT / "sink.arrows")
SINK_FILE = IOBase(ROOT / "sink.parquet")
AVRO_OPTIONS = IOBase(ROOT / "options.avro").record_options()
AVRO_SYNC_MARKER = b"0123456789abcdef"


@dataclass(frozen=True)
class Benchmark:
    """One measured operation and the unit its throughput is reported in."""

    name: str
    operation: Callable[[], object]
    units: int
    unit_name: str
    prepare: Callable[[], object] | None = None


def _materialized(handle: IOBase, field: object | None) -> int:
    """Read every batch and report the bytes the read actually built."""
    options = handle.record_options()
    if field is not None:
        options.field = field
    return sum(batch.nbytes for batch in handle.read_arrow_reader(options=options))


def _write_stream() -> object:
    SINK_STREAM.overwrite_arrow_reader(TABLE.to_reader())
    return SINK_STREAM.size


def _write_file() -> object:
    SINK_FILE.overwrite_arrow_reader(TABLE.to_reader())
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
    return STREAM.read_arrow_reader().read_all().num_rows


def _pyarrow_ipc_baseline() -> object:
    # The same work through PyArrow's own writer, to the same kind of
    # destination, so the two numbers are comparable.
    with pa.OSFile(str(ROOT / "baseline.arrows"), "wb") as sink:
        with pa.ipc.new_stream(sink, SCHEMA) as writer:
            for batch in BATCHES:
                writer.write_batch(batch)
    return (ROOT / "baseline.arrows").stat().st_size


def _pyarrow_parquet_write_baseline() -> object:
    # The same rows through PyArrow's own Parquet writer, so the parquet
    # write row above has a trusted number beside it.
    import pyarrow.parquet as pq

    pq.write_table(TABLE, ROOT / "baseline.parquet")
    return (ROOT / "baseline.parquet").stat().st_size


def _pyarrow_parquet_read_baseline() -> object:
    import pyarrow.parquet as pq

    return pq.read_table(ROOT / "baseline.parquet").num_rows


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
RECORD_BATCH = TABLE.combine_chunks().to_batches(max_chunksize=ROW_COUNT)[0]
MERGE_OPTIONS = SINK_FILE.record_options()
MERGE_OPTIONS.merge_by_names = ["id"]
COMMIT_OPTIONS = SINK_FILE.record_options()
COMMIT_OPTIONS.commit_row_size = BATCH_SIZE
COMMIT_MERGE_OPTIONS = SINK_FILE.record_options()
COMMIT_MERGE_OPTIONS.commit_row_size = BATCH_SIZE
COMMIT_MERGE_OPTIONS.merge_by_names = ["id"]


def _prepare_existing() -> object:
    """Reset append/merge benchmarks outside their measured operation."""
    SINK_FILE.overwrite_arrow_table(TABLE)
    return SINK_FILE.size


def _arrow_input(shape: str) -> object:
    if shape == "arrow_reader":
        return pa.RecordBatchReader.from_batches(SCHEMA, iter(BATCHES))
    if shape == "arrow_table":
        return TABLE
    if shape == "arrow_record_batch":
        return RECORD_BATCH
    if shape == "records":
        return iter(ROW_MAPPINGS)
    raise AssertionError(shape)


def _write_shape(intent: str, shape: str, *, commit: bool = False) -> object:
    method = getattr(SINK_FILE, f"{intent}_{shape}")
    source = _arrow_input(shape)
    if commit:
        options = COMMIT_MERGE_OPTIONS if intent == "merge" else COMMIT_OPTIONS
        method(source, options=options)
    elif intent == "merge":
        method(source, options=MERGE_OPTIONS)
    else:
        method(source)
    return SINK_FILE.size


def _write_frame(intent: str, package: str, whole: bool) -> object:
    frame = PANDAS_FRAME if package == "pandas" else POLARS_FRAME
    suffix = f"{package}_frame" if whole else package
    method = getattr(SINK_FILE, f"{intent}_{suffix}")
    source = frame if whole else (frame,)
    if intent == "merge":
        method(source, options=MERGE_OPTIONS)
    else:
        method(source)
    return SINK_FILE.size


def _read_pandas_frame() -> object:
    return len(FILE.read_pandas_frame())


def _read_polars_frame() -> object:
    return FILE.read_polars_frame().height


def _read_records() -> object:
    return sum(1 for _ in FILE.read_records())


def _fresh_row_size() -> object:
    return IOBase(ROOT / "trades.parquet").row_size


def _fresh_column_size() -> object:
    return IOBase(ROOT / "trades.parquet").column_size


def _cached_row_size() -> object:
    return FILE.row_size


def _cached_column_size() -> object:
    return FILE.column_size


def _record_options() -> object:
    return FILE.record_options()


def _avro_option_values() -> object:
    return AVRO_OPTIONS.block_codec, AVRO_OPTIONS.sync_marker


def _set_avro_option_values() -> object:
    AVRO_OPTIONS.block_codec = "deflate"
    AVRO_OPTIONS.sync_marker = AVRO_SYNC_MARKER
    return AVRO_OPTIONS.sync_marker


def _read_arrow_field() -> object:
    return FILE.read_arrow_field()


def _read_parquet_statistics() -> object:
    """Cross the native footer DTO through the shared Value projection."""
    return FILE.read_parquet_statistics()


def _read_parquet_geospatial_statistics() -> object:
    """Cross one projected WKB scan and its native statistics record."""
    return GEO_FILE.read_parquet_geospatial_statistics("shape")


def _is_io() -> object:
    return FILE.is_io()


def _kind() -> object:
    return FILE.kind


def _write_shape_mode(mode: str, shape: str) -> object:
    method = getattr(SINK_FILE, f"write_{shape}")
    source = _arrow_input(shape)
    if mode == "merge":
        method(source, mode, options=MERGE_OPTIONS)
    else:
        method(source, mode)
    return SINK_FILE.size


def _write_frame_mode(mode: str, package: str, whole: bool) -> object:
    frame = PANDAS_FRAME if package == "pandas" else POLARS_FRAME
    suffix = f"{package}_frame" if whole else package
    method = getattr(SINK_FILE, f"write_{suffix}")
    source = frame if whole else (frame,)
    if mode == "merge":
        method(source, mode, options=MERGE_OPTIONS)
    else:
        method(source, mode)
    return SINK_FILE.size


INTENT_BENCHMARKS = tuple(
    Benchmark(
        f"parquet {intent} {shape.replace('_', ' ')}",
        lambda intent=intent, shape=shape: _write_shape(intent, shape),
        ROW_COUNT,
        "row",
        _prepare_existing if intent != "overwrite" else None,
    )
    for shape in ("arrow_reader", "arrow_table", "arrow_record_batch", "records")
    for intent in ("overwrite", "append", "merge")
)

COMMIT_BENCHMARKS = tuple(
    Benchmark(
        f"parquet {intent} {shape.replace('_', ' ')} commit {BATCH_SIZE}",
        lambda intent=intent, shape=shape: _write_shape(intent, shape, commit=True),
        ROW_COUNT,
        "row",
        _prepare_existing if intent != "overwrite" else None,
    )
    for shape in ("arrow_reader", "records")
    for intent in ("overwrite", "append", "merge")
)

FRAME_BENCHMARKS = tuple(
    Benchmark(
        f"{package} {intent} {'frame' if whole else 'frames'}",
        lambda intent=intent, package=package, whole=whole: _write_frame(
            intent, package, whole
        ),
        ROW_COUNT,
        "row",
        _prepare_existing if intent != "overwrite" else None,
    )
    for package, frame in (("pandas", PANDAS_FRAME), ("polars", POLARS_FRAME))
    if frame is not None
    for whole in (False, True)
    for intent in ("overwrite", "append", "merge")
)

MODE_BENCHMARKS = tuple(
    Benchmark(
        f"generic {mode} {shape.replace('_', ' ')}",
        lambda mode=mode, shape=shape: _write_shape_mode(mode, shape),
        ROW_COUNT,
        "row",
        _prepare_existing if mode != "overwrite" else None,
    )
    for shape in ("arrow_reader", "arrow_table", "arrow_record_batch", "records")
    for mode in ("overwrite", "append", "merge")
)

MODE_FRAME_BENCHMARKS = tuple(
    Benchmark(
        f"generic {package} {mode} {'frame' if whole else 'frames'}",
        lambda mode=mode, package=package, whole=whole: _write_frame_mode(
            mode, package, whole
        ),
        ROW_COUNT,
        "row",
        _prepare_existing if mode != "overwrite" else None,
    )
    for package, frame in (("pandas", PANDAS_FRAME), ("polars", POLARS_FRAME))
    if frame is not None
    for whole in (False, True)
    for mode in ("overwrite", "append", "merge")
)


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
        Benchmark("parquet read records", _read_records, ROW_COUNT, "row"),
        Benchmark("parquet row size fresh", _fresh_row_size, 1, "lookup"),
        Benchmark("parquet column size fresh", _fresh_column_size, 1, "lookup"),
        Benchmark("parquet row size cached", _cached_row_size, 1, "lookup"),
        Benchmark("parquet column size cached", _cached_column_size, 1, "lookup"),
        Benchmark("parquet record options", _record_options, 1, "lookup"),
        Benchmark("avro option values", _avro_option_values, 1, "lookup"),
        Benchmark("avro option setters", _set_avro_option_values, 1, "lookup"),
        Benchmark("parquet read arrow field", _read_arrow_field, 1, "lookup"),
        Benchmark("parquet read statistics", _read_parquet_statistics, 1, "lookup"),
        Benchmark(
            "parquet read geospatial stats",
            _read_parquet_geospatial_statistics,
            GEO_ROW_COUNT,
            "row",
        ),
        Benchmark("parquet is io", _is_io, 1, "lookup"),
        Benchmark("parquet kind", _kind, 1, "lookup"),
        Benchmark("PyArrow IPC write baseline", _pyarrow_ipc_baseline, ROW_COUNT, "row"),
        Benchmark(
            "PyArrow parquet write baseline",
            _pyarrow_parquet_write_baseline,
            ROW_COUNT,
            "row",
        ),
        Benchmark(
            "PyArrow parquet read baseline",
            _pyarrow_parquet_read_baseline,
            ROW_COUNT,
            "row",
        ),
        *INTENT_BENCHMARKS,
        *COMMIT_BENCHMARKS,
        *FRAME_BENCHMARKS,
        *MODE_BENCHMARKS,
        *MODE_FRAME_BENCHMARKS,
        Benchmark("pandas read frame", _read_pandas_frame, ROW_COUNT, "row")
        if PANDAS_FRAME is not None
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
    if benchmark.prepare is not None:
        benchmark.prepare()
        benchmark.operation()
        samples = []
        for _ in range(repeat):
            benchmark.prepare()
            gc.collect()
            started = timeit.default_timer()
            benchmark.operation()
            samples.append(timeit.default_timer() - started)
        return statistics.median(samples), min(samples), 1

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
    parser.add_argument(
        "--filter",
        action="append",
        default=[],
        help="run benchmark names containing any supplied text",
    )
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")
    selected = tuple(
        benchmark
        for benchmark in BENCHMARKS
        if not arguments.filter
        or any(fragment in benchmark.name for fragment in arguments.filter)
    )
    if not selected:
        parser.error("--filter matched no benchmark")

    try:
        STREAM.overwrite_arrow_table(TABLE)
        FILE.overwrite_arrow_table(TABLE)
        GEO_FILE.overwrite_arrow_table(GEO_TABLE)
        FILE.open()

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
            for benchmark in selected:
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
        for handle in (STREAM, FILE, GEO_FILE, SINK_STREAM, SINK_FILE):
            handle.close()
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
