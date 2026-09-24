"""The Arrow value boundary: every columnar runtime in as a ``Serie`` or a
``SerieReader``, every shape out.

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

import argparse
import atexit
import gc
import importlib
import json
import pathlib
import shutil
import statistics
import sys
import tempfile
import timeit
from collections.abc import Callable

import pyarrow as pa

from yggdryl import ArrowCastPlan, ChunkedSerie, Field, IOBase, Scalar, Serie, SerieReader

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
CHUNKED_TABLE = pa.Table.from_batches(
    [BATCH.slice(0, ROW_COUNT // 2), BATCH.slice(ROW_COUNT // 2)]
)
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
# A held plan is what a loop over many columns of one layout keeps, so its row
# measures the per-column half alone; the one-shot doors compile per call.
COLUMN_PLAN = ArrowCastPlan(SCHEMA.field("size"), DECLARED_COLUMN)
BATCH_PLAN = ArrowCastPlan(SCHEMA, DECLARED_ROOT)

# A held shape shares its buffers back, so one value answers every export as
# often as it is asked. A stream would be spent by the first measured call.
HELD_BATCH = Serie.from_(BATCH)
HELD_COLUMN = Serie.from_(COLUMN)
HELD_SCALAR = Serie.from_(SCALAR)
# The same two halves kept apart: a chunked array and a table of two batches.
HELD_CHUNKED = ChunkedSerie.from_(CHUNKED)
HELD_CHUNKED_TABLE = ChunkedSerie.from_(CHUNKED_TABLE)
# A reader wraps a held leaf as a record before applying its declared root.
# Naming this fixture explicitly keeps the cast independent of Array inference.
HELD_READER_COLUMN = Serie.from_arrow_array(
    COLUMN, Field("size", "int64", nullable=False)
)
READER_COLUMN_ROOT = Field("row", "struct<size: float64 not null>", nullable=False)
NATIVE_SCALAR = Scalar.from_(125)
CONCRETE_ROWS = (1, 2, 3, 4)

MAP_TYPE = pa.map_(pa.string(), pa.string(), keys_sorted=True)
MAP_SCHEMA = pa.schema(
    [pa.field("identifiers", pa.struct([pa.field("values", MAP_TYPE)]))],
    metadata={b"fixture": b"sorted-map-exchange"},
)
MAP_BATCH = pa.RecordBatch.from_arrays(
    [
        pa.array(
            [{"values": [("orderid", "ORDER-000001")]}] * ROW_COUNT,
            type=MAP_SCHEMA.field(0).type,
        )
    ],
    schema=MAP_SCHEMA,
)
HELD_MAP_BATCH = Serie.from_(MAP_BATCH)

STORE = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-arrow-bench-"))
# The store is made at import, before any argument is read, so its removal is
# registered here too: `--help` and a refused argument both exit before `main`
# reaches the block that would otherwise clean it up.
atexit.register(shutil.rmtree, STORE, ignore_errors=True)
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
    # The core writes a record as a sorted, separator-free object, so the
    # baseline writes the same bytes: a 10% larger document with a different
    # key order would make the pair a comparison of two documents rather than
    # of two ways to write one. `main` asserts the two agree byte for byte.
    return BASELINE_LINES.write_text(
        "".join(
            f"{json.dumps(row, separators=(',', ':'), sort_keys=True)}\n"
            for row in TEXT_ROWS
        ),
        encoding="utf-8",
        newline="\n",
    )


def _read_jsonl_baseline() -> int:
    decoded = [
        json.loads(line)
        for line in BASELINE_LINES.read_text(encoding="utf-8").splitlines()
    ]
    return pa.Table.from_pylist(decoded, schema=SCHEMA).num_rows


def _read_stream_value() -> int:
    return STREAM.read_arrow().into_arrow_reader().read_all().num_rows


def _read_lines_value() -> int:
    return len(Serie.from_(LINES.read_arrow(field=TEXT_ROOT)))


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
        ("from Table (yggdryl)", lambda: SerieReader.from_(TABLE), small),
        ("from Table (pyarrow)", lambda: TABLE.to_reader(), small),
        # A held container has no PyArrow counterpart to subtract: nothing else
        # imports it across the C Data Interface, so the number beside it is
        # the whole crossing rather than a difference. Rows without a
        # `(pyarrow)` partner are read that way.
        ("from RecordBatch", lambda: Serie.from_(BATCH), small),
        ("from Array", lambda: Serie.from_(COLUMN), small),
        ("from ChunkedArray (yggdryl)", lambda: Serie.from_(CHUNKED), bulk),
        # `Serie.from_` lands each chunk and joins them once - what
        # `ChunkedSerie.into_serie` is - so combining is the same join minus
        # the crossing.
        ("from ChunkedArray (pyarrow)", lambda: CHUNKED.combine_chunks(), bulk),
        # Kept apart, each chunk or batch crosses on its own, buffers shared,
        # and nothing is joined: no PyArrow work stands beside it.
        (
            "ChunkedSerie from ChunkedArray",
            lambda: ChunkedSerie.from_(CHUNKED),
            small,
        ),
        (
            "ChunkedSerie from Table of two batches",
            lambda: ChunkedSerie.from_(CHUNKED_TABLE),
            small,
        ),
        ("from Scalar", lambda: Serie.from_(SCALAR), small),
        (
            "from RecordBatchReader (yggdryl)",
            lambda: SerieReader.from_(_reader()),
            small,
        ),
        ("from RecordBatchReader (pyarrow)", _reader, small),
        ("SerieReader from Python scalar", lambda: SerieReader.from_(125), small),
        (
            "SerieReader from native Scalar",
            lambda: SerieReader.from_(NATIVE_SCALAR),
            small,
        ),
        (
            "SerieReader from concrete tuple",
            lambda: SerieReader.from_(CONCRETE_ROWS),
            small,
        ),
        (
            "SerieReader Python scalar first batch",
            lambda: next(SerieReader.from_(125)),
            small,
        ),
    ]
    if pandas is not None:
        cases += [
            (
                "from pandas DataFrame (yggdryl)",
                lambda: Serie.from_(PANDAS_FRAME),
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
                lambda: Serie.from_(POLARS_FRAME),
                bulk,
            ),
            ("from polars DataFrame (polars)", POLARS_FRAME.to_arrow, bulk),
        ]
    if numpy is not None:
        cases += [
            (
                "from numpy array (yggdryl)",
                lambda: Serie.from_(NUMPY_COLUMN),
                bulk,
            ),
            ("from numpy array (pyarrow)", lambda: pa.array(NUMPY_COLUMN), bulk),
            (
                "from numpy records (yggdryl)",
                lambda: Serie.from_(NUMPY_RECORDS),
                bulk,
            ),
            ("from numpy records (pyarrow)", _numpy_records_baseline, bulk),
        ]
    cases += [
        (
            "from Array, declared (yggdryl)",
            lambda: Serie.from_(COLUMN, DECLARED_COLUMN),
            bulk,
        ),
        ("from Array, declared (pyarrow)", lambda: COLUMN.cast(pa.float64()), bulk),
        (
            "from RecordBatch, declared (yggdryl)",
            lambda: Serie.from_(BATCH, DECLARED_ROOT),
            bulk,
        ),
        (
            "from RecordBatch, declared (pyarrow)",
            lambda: BATCH.cast(DECLARED_SCHEMA),
            bulk,
        ),
        ("cast a held column", lambda: HELD_COLUMN.cast(DECLARED_COLUMN), bulk),
        # One plan over both chunks, beside PyArrow's cast of the same two.
        (
            "ChunkedSerie.cast, declared (yggdryl)",
            lambda: HELD_CHUNKED.cast(DECLARED_COLUMN),
            bulk,
        ),
        (
            "ChunkedSerie.cast, declared (pyarrow)",
            lambda: CHUNKED.cast(pa.float64()),
            bulk,
        ),
        (
            "ArrowCastPlan.apply, ChunkedArray",
            lambda: COLUMN_PLAN.apply(CHUNKED),
            bulk,
        ),
        # Construction plans the record cast; only the first-batch row executes it.
        (
            "SerieReader held column, declared root",
            lambda: SerieReader.from_(HELD_READER_COLUMN, READER_COLUMN_ROOT),
            small,
        ),
        (
            "SerieReader held declared first batch",
            lambda: next(SerieReader.from_(HELD_READER_COLUMN, READER_COLUMN_ROOT)),
            bulk,
        ),
        # The Serie doors pair with the two PyArrow casts above: the same
        # array or batch, cast onto the same declared field.
        (
            "Serie.from_arrow_array, declared",
            lambda: Serie.from_arrow_array(COLUMN, DECLARED_COLUMN),
            bulk,
        ),
        ("ArrowCastPlan.apply, Array", lambda: COLUMN_PLAN.apply(COLUMN), bulk),
        (
            "Serie.from_arrow_batch, declared",
            lambda: Serie.from_arrow_batch(BATCH, DECLARED_ROOT),
            bulk,
        ),
        ("ArrowCastPlan.apply, RecordBatch", lambda: BATCH_PLAN.apply(BATCH), bulk),
        (
            "SerieReader drain, declared",
            lambda: list(SerieReader.from_arrow_reader(TABLE, DECLARED_ROOT)),
            bulk,
        ),
        ("into_arrow_reader (yggdryl)", lambda: HELD_BATCH.into_arrow_reader(), small),
        (
            "into_arrow_reader (pyarrow)",
            lambda: pa.RecordBatchReader.from_batches(SCHEMA, iter([BATCH])),
            small,
        ),
        ("into_arrow_batch", lambda: HELD_BATCH.into_arrow_batch(), small),
        ("sorted Map batch export", lambda: HELD_MAP_BATCH.into_arrow_batch(), small),
        (
            "sorted Map reader construction",
            lambda: HELD_MAP_BATCH.into_arrow_reader(),
            small,
        ),
        (
            "sorted Map reader first batch",
            lambda: HELD_MAP_BATCH.into_arrow_reader().read_next_batch(),
            small,
        ),
        ("into_arrow_table (yggdryl)", lambda: HELD_BATCH.into_arrow_table(), small),
        (
            "into_arrow_table (pyarrow)",
            lambda: pa.Table.from_batches([BATCH], schema=SCHEMA),
            small,
        ),
        ("into_arrow_array", lambda: HELD_COLUMN.into_arrow_array(), small),
        (
            "into_arrow_chunked_array (yggdryl)",
            lambda: HELD_CHUNKED.into_arrow_chunked_array(),
            small,
        ),
        (
            "into_arrow_chunked_array (pyarrow)",
            lambda: pa.chunked_array(CHUNKED.chunks, type=CHUNKED.type),
            small,
        ),
        (
            "ChunkedSerie into_arrow_table (yggdryl)",
            lambda: HELD_CHUNKED_TABLE.into_arrow_table(),
            small,
        ),
        (
            "ChunkedSerie into_arrow_table (pyarrow)",
            lambda: pa.Table.from_batches(CHUNKED_TABLE.to_batches(), schema=SCHEMA),
            small,
        ),
        ("ChunkedSerie into_serie (yggdryl)", lambda: HELD_CHUNKED.into_serie(), bulk),
        ("ChunkedSerie into_serie (pyarrow)", lambda: CHUNKED.combine_chunks(), bulk),
        ("into_arrow_scalar (yggdryl)", lambda: HELD_SCALAR.into_arrow_scalar(), small),
        ("into_arrow_scalar (pyarrow)", lambda: ONE_ROW[0], small),
    ]
    if pandas is not None:
        cases += [
            ("into_pandas (yggdryl)", lambda: HELD_BATCH.into_pandas(), bulk),
            ("into_pandas (pyarrow)", lambda: BATCH.to_pandas(), bulk),
        ]
    if polars is not None:
        cases += [
            ("into_polars (yggdryl)", lambda: HELD_BATCH.into_polars(), bulk),
            ("into_polars (polars)", lambda: polars.from_arrow(BATCH), bulk),
        ]
    if numpy is not None:
        cases += [
            ("into_numpy (yggdryl)", lambda: HELD_COLUMN.into_numpy(), bulk),
            (
                "into_numpy (pyarrow)",
                lambda: COLUMN.to_numpy(zero_copy_only=False),
                bulk,
            ),
        ]
    return (
        *cases,
        ("into_scalar", lambda: HELD_BATCH.into_scalar(), bulk),
        ("as_py", lambda: HELD_BATCH.as_py(), bulk),
        ("len", lambda: len(HELD_BATCH), small),
        ("field", lambda: HELD_BATCH.field, small),
        (
            "write_arrow arrows (yggdryl)",
            lambda: STREAM.write_arrow(TABLE),
            io,
        ),
        ("write_arrow arrows (pyarrow)", _write_ipc_baseline, io),
        ("read_arrow arrows (yggdryl)", _read_stream_value, io),
        ("read_arrow arrows (pyarrow)", _read_ipc_baseline, io),
        (
            f"write_arrow jsonl {TEXT_ROW_COUNT:,} (yggdryl)",
            lambda: LINES.write_arrow(TEXT_TABLE),
            io,
        ),
        (
            f"write_arrow jsonl {TEXT_ROW_COUNT:,} (stdlib json)",
            _write_jsonl_baseline,
            io,
        ),
        (f"read_arrow jsonl {TEXT_ROW_COUNT:,} (yggdryl)", _read_lines_value, io),
        (
            f"read_arrow jsonl {TEXT_ROW_COUNT:,} (stdlib json)",
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
        reader = SerieReader.from_(HELD_READER_COLUMN, READER_COLUMN_ROOT)
        assert reader.field == READER_COLUMN_ROOT
        (declared,) = list(reader)
        assert declared.child("size").into_arrow_array().equals(COLUMN.cast(pa.float64()))
        for value in (125, NATIVE_SCALAR, CONCRETE_ROWS):
            assert list(SerieReader.from_(value)) == list(
                SerieReader.from_serie(Serie.from_(value))
            )
        for exported in (
            HELD_MAP_BATCH.into_arrow_batch(),
            HELD_MAP_BATCH.into_arrow_reader().read_next_batch(),
        ):
            assert exported.equals(MAP_BATCH, check_metadata=True)
        STREAM.write_arrow(TABLE)
        LINES.write_arrow(TEXT_TABLE)
        _write_jsonl_baseline()
        _write_ipc_baseline()
        # A pair that reads two different documents measures the documents,
        # not the readers, so the two are required to be the same bytes.
        if LINES.read_bytes() != BASELINE_LINES.read_bytes():
            raise SystemExit(
                "the JSON Lines baseline document differs from the one the core "
                "wrote, so the text pair would compare two documents"
            )
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
