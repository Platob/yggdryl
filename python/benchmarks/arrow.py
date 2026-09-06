"""The Arrow value boundary: every columnar runtime in, every shape out.

Run after ``maturin develop`` with::

    python benchmarks/arrow.py --iterations 10000

Each crossing that has one is paired with the most direct PyArrow-only
expression a caller would otherwise write over the same source, so the
difference between a row's ``(yggdryl)`` and its ``(pyarrow)`` number is what
the wrapper itself costs. A crossing whose only honest counterpart is a private
C Data round trip is measured alone rather than against an invented baseline.

The declared-field rows use a held batch rather than a table: a table crosses
as a stream, whose cast compiles a plan and pulls nothing, so pairing it with
PyArrow's eager cast would compare two different amounts of work.

pandas, polars, and NumPy are optional at this boundary exactly as they are in
the binding. A library that is not installed leaves its rows out and is named
once, rather than reported as zero.
"""

from __future__ import annotations

import os
import sys

# A script's own directory comes first on `sys.path`, and the sibling benchmark
# `types.py` carries the name `enum` imports from the standard library. On an
# interpreter that has not already loaded the real `types`, the first import
# below would resolve to that sibling, so the directory leaves first; nothing
# here is loaded from it.
_SCRIPT_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [
    entry
    for entry in sys.path
    if os.path.abspath(entry or os.curdir) != _SCRIPT_DIRECTORY
]

import argparse  # noqa: E402
import gc  # noqa: E402
import importlib  # noqa: E402
import json  # noqa: E402
import pathlib  # noqa: E402
import shutil  # noqa: E402
import statistics  # noqa: E402
import tempfile  # noqa: E402
import timeit  # noqa: E402
from collections.abc import Callable  # noqa: E402

import pyarrow as pa  # noqa: E402

from yggdryl import ArrowValue, Field, IOBase  # noqa: E402

ROW_COUNT = 4_096
# `Limits::default().max_documents()` is 1,024, and JSON Lines yields one
# document per row, so the structured-text rows measure the widest corpus a
# default read accepts rather than a corpus it refuses.
TEXT_ROW_COUNT = 1_024

SCHEMA = pa.schema(
    [
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("size", pa.int64(), nullable=False),
    ]
)
BATCH = pa.record_batch(
    {"symbol": ["AAPL"] * ROW_COUNT, "size": list(range(ROW_COUNT))}, schema=SCHEMA
)
TABLE = pa.Table.from_batches([BATCH])
TEXT_TABLE = TABLE.slice(0, TEXT_ROW_COUNT)
TEXT_ROWS = TEXT_TABLE.to_pylist()
COLUMN = BATCH.column("size")
CHUNKED = pa.chunked_array([COLUMN[: ROW_COUNT // 2], COLUMN[ROW_COUNT // 2 :]])
SCALAR = pa.scalar(125, pa.int64())
ONE_ROW = pa.array([125], type=pa.int64())

# The declared field is built once: parsing an expression is measured by the
# field benchmarks, and what these rows compare is the cast.
DECLARED_COLUMN = Field("size", "float64", nullable=False)
DECLARED_ROOT = Field(
    "row",
    "struct<symbol: utf8 not null, size: float64 not null>",
    nullable=False,
)
# A structured-text read types the document it parses, so the root it is given
# is the one the rows were written under, not the cast target above.
TEXT_ROOT = Field(
    "row",
    "struct<symbol: utf8 not null, size: int64 not null>",
    nullable=False,
)
DECLARED_SCHEMA = pa.schema(
    [
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("size", pa.float64(), nullable=False),
    ]
)

# A held shape shares its buffers back, so one value answers every export as
# often as it is asked. A stream would be spent by the first measured call.
HELD_BATCH = ArrowValue.from_py(BATCH)
HELD_COLUMN = ArrowValue.from_py(COLUMN)
HELD_SCALAR = ArrowValue.from_py(SCALAR)

STORE = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-arrow-bench-"))
STREAM = IOBase(STORE / "quotes.arrows")
LINES = IOBase(STORE / "quotes.jsonl")
BASELINE_STREAM = STORE / "baseline.arrows"
BASELINE_LINES = STORE / "baseline.jsonl"


def _optional(package: str) -> object | None:
    """Import an optional frame or array library, or report its absence."""
    try:
        return importlib.import_module(package)
    except ImportError:
        return None


pandas = _optional("pandas")
polars = _optional("polars")
numpy = _optional("numpy")

PANDAS_FRAME = None if pandas is None else TABLE.to_pandas()
POLARS_FRAME = None if polars is None else polars.from_arrow(TABLE)
NUMPY_COLUMN = None if numpy is None else COLUMN.to_numpy()
NUMPY_RECORDS = (
    None
    if numpy is None
    else numpy.array(
        [("AAPL", size) for size in range(ROW_COUNT)],
        dtype=[("symbol", "U4"), ("size", "i8")],
    )
)


def _reader() -> pa.RecordBatchReader:
    """One fresh reader; a reader is one-shot, so it is built per measurement."""
    return pa.RecordBatchReader.from_batches(SCHEMA, iter([BATCH]))


def _numpy_records_baseline() -> pa.RecordBatch:
    # NumPy interleaves a record dtype's members in one buffer, so PyArrow
    # converts them one column at a time, which is what the binding does too.
    names = list(NUMPY_RECORDS.dtype.names)
    return pa.RecordBatch.from_arrays(
        [pa.array(NUMPY_RECORDS[name]) for name in names], names=names
    )


def _write_ipc_baseline() -> int:
    with pa.OSFile(str(BASELINE_STREAM), "wb") as sink:
        with pa.ipc.new_stream(sink, SCHEMA) as writer:
            writer.write_batch(BATCH)
    return BASELINE_STREAM.stat().st_size


def _read_ipc_baseline() -> int:
    with pa.OSFile(str(BASELINE_STREAM), "rb") as source:
        with pa.ipc.open_stream(source) as reader:
            return reader.read_all().num_rows


def _write_jsonl_baseline() -> int:
    return BASELINE_LINES.write_text(
        "".join(f"{json.dumps(row)}\n" for row in TEXT_ROWS), encoding="utf-8"
    )


def _read_jsonl_baseline() -> int:
    decoded = [
        json.loads(line)
        for line in BASELINE_LINES.read_text(encoding="utf-8").splitlines()
    ]
    return pa.Table.from_pylist(decoded, schema=SCHEMA).num_rows


def _read_stream_value() -> int:
    return STREAM.read_arrow_value().into_arrow_table().num_rows


def _read_lines_value() -> int:
    return LINES.read_arrow_value(TEXT_ROOT).row_size


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    nanoseconds = statistics.median(samples) * 1_000_000_000 / iterations
    print(f"{name:44} {nanoseconds:12.1f} ns/op")


def _cases(
    small: int, bulk: int, io: int
) -> tuple[tuple[str, Callable[[], object], int], ...]:
    """Every measured row, in ingest, declare, export, read, handle order."""
    cases: list[tuple[str, Callable[[], object], int]] = [
        # A table may hold many chunks, so it crosses over the C stream that
        # `to_reader` is the PyArrow spelling of: neither side pulls a batch.
        ("from Table (yggdryl)", lambda: ArrowValue.from_py(TABLE), small),
        ("from Table (pyarrow)", TABLE.to_reader, small),
        ("from RecordBatch (yggdryl)", lambda: ArrowValue.from_py(BATCH), small),
        (
            "from RecordBatch (pyarrow)",
            lambda: pa.Table.from_batches([BATCH], schema=SCHEMA),
            small,
        ),
        ("from Array (yggdryl)", lambda: ArrowValue.from_py(COLUMN), small),
        ("from Array (pyarrow)", lambda: pa.chunked_array([COLUMN]), small),
        ("from ChunkedArray (yggdryl)", lambda: ArrowValue.from_py(CHUNKED), bulk),
        ("from ChunkedArray (pyarrow)", CHUNKED.combine_chunks, bulk),
        ("from Scalar (yggdryl)", lambda: ArrowValue.from_py(SCALAR), small),
        ("from Scalar (pyarrow)", lambda: pa.array([SCALAR]), small),
        (
            "from RecordBatchReader (yggdryl)",
            lambda: ArrowValue.from_py(_reader()),
            small,
        ),
        ("from RecordBatchReader (pyarrow)", _reader, small),
    ]
    if pandas is not None:
        cases += [
            (
                "from pandas DataFrame (yggdryl)",
                lambda: ArrowValue.from_py(PANDAS_FRAME),
                bulk,
            ),
            (
                "from pandas DataFrame (pyarrow)",
                lambda: pa.Table.from_pandas(PANDAS_FRAME),
                bulk,
            ),
        ]
    if polars is not None:
        cases += [
            (
                "from polars DataFrame (yggdryl)",
                lambda: ArrowValue.from_py(POLARS_FRAME),
                bulk,
            ),
            ("from polars DataFrame (polars)", POLARS_FRAME.to_arrow, bulk),
        ]
    if numpy is not None:
        cases += [
            (
                "from numpy array (yggdryl)",
                lambda: ArrowValue.from_py(NUMPY_COLUMN),
                bulk,
            ),
            ("from numpy array (pyarrow)", lambda: pa.array(NUMPY_COLUMN), bulk),
            (
                "from numpy records (yggdryl)",
                lambda: ArrowValue.from_py(NUMPY_RECORDS),
                bulk,
            ),
            ("from numpy records (pyarrow)", _numpy_records_baseline, bulk),
        ]
    cases += [
        (
            "from Array, declared (yggdryl)",
            lambda: ArrowValue.from_py(COLUMN, DECLARED_COLUMN),
            bulk,
        ),
        ("from Array, declared (pyarrow)", lambda: COLUMN.cast(pa.float64()), bulk),
        (
            "from RecordBatch, declared (yggdryl)",
            lambda: ArrowValue.from_py(BATCH, DECLARED_ROOT),
            bulk,
        ),
        (
            "from RecordBatch, declared (pyarrow)",
            lambda: BATCH.cast(DECLARED_SCHEMA),
            bulk,
        ),
        ("cast a held column", lambda: HELD_COLUMN.cast(DECLARED_COLUMN), bulk),
        ("into_arrow_reader (yggdryl)", HELD_BATCH.into_arrow_reader, small),
        (
            "into_arrow_reader (pyarrow)",
            lambda: pa.RecordBatchReader.from_batches(SCHEMA, iter([BATCH])),
            small,
        ),
        ("into_arrow_batch", HELD_BATCH.into_arrow_batch, small),
        ("into_arrow_table (yggdryl)", HELD_BATCH.into_arrow_table, small),
        (
            "into_arrow_table (pyarrow)",
            lambda: pa.Table.from_batches([BATCH], schema=SCHEMA),
            small,
        ),
        ("into_arrow_array", HELD_COLUMN.into_arrow_array, small),
        ("into_arrow_scalar (yggdryl)", HELD_SCALAR.into_arrow_scalar, small),
        ("into_arrow_scalar (pyarrow)", lambda: ONE_ROW[0], small),
    ]
    if pandas is not None:
        cases += [
            ("into_pandas (yggdryl)", HELD_BATCH.into_pandas, bulk),
            ("into_pandas (pyarrow)", BATCH.to_pandas, bulk),
        ]
    if polars is not None:
        cases += [
            ("into_polars (yggdryl)", HELD_BATCH.into_polars, bulk),
            ("into_polars (polars)", lambda: polars.from_arrow(BATCH), bulk),
        ]
    if numpy is not None:
        cases += [
            ("into_numpy (yggdryl)", HELD_COLUMN.into_numpy, bulk),
            (
                "into_numpy (pyarrow)",
                lambda: COLUMN.to_numpy(zero_copy_only=False),
                bulk,
            ),
        ]
    return (
        *cases,
        ("into_scalar", HELD_BATCH.into_scalar, bulk),
        ("as_py", HELD_BATCH.as_py, bulk),
        ("shape", lambda: HELD_BATCH.shape, small),
        ("row_size", lambda: HELD_BATCH.row_size, small),
        ("field", lambda: HELD_BATCH.field, small),
        (
            "write_arrow_value arrows (yggdryl)",
            lambda: STREAM.write_arrow_value(TABLE),
            io,
        ),
        ("write_arrow_value arrows (pyarrow)", _write_ipc_baseline, io),
        ("read_arrow_value arrows (yggdryl)", _read_stream_value, io),
        ("read_arrow_value arrows (pyarrow)", _read_ipc_baseline, io),
        (
            f"write_arrow_value jsonl {TEXT_ROW_COUNT:,} (yggdryl)",
            lambda: LINES.write_arrow_value(TEXT_TABLE),
            io,
        ),
        (
            f"write_arrow_value jsonl {TEXT_ROW_COUNT:,} (stdlib json)",
            _write_jsonl_baseline,
            io,
        ),
        (f"read_arrow_value jsonl {TEXT_ROW_COUNT:,} (yggdryl)", _read_lines_value, io),
        (
            f"read_arrow_value jsonl {TEXT_ROW_COUNT:,} (stdlib json)",
            _read_jsonl_baseline,
            io,
        ),
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=10_000)
    arguments = parser.parse_args()
    if arguments.iterations < 1:
        parser.error("--iterations must be positive")

    small = arguments.iterations
    bulk = max(1, small // 100)
    io = max(1, small // 1_000)
    for package, library in (("pandas", pandas), ("polars", polars), ("numpy", numpy)):
        if library is None:
            print(f"{package} is not installed; its rows are skipped")

    print(
        f"Python {sys.version.split()[0]}, PyArrow {pa.__version__}; "
        f"{ROW_COUNT:,} rows, median of 7"
    )
    try:
        STREAM.write_arrow_value(TABLE)
        LINES.write_arrow_value(TEXT_TABLE)
        _write_jsonl_baseline()
        _write_ipc_baseline()
        gc.disable()
        try:
            for name, operation, iterations in _cases(small, bulk, io):
                _measure(name, operation, iterations)
        finally:
            gc.enable()
    finally:
        shutil.rmtree(STORE, ignore_errors=True)


if __name__ == "__main__":
    main()
