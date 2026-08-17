"""Reproducible Arrow/tabular baselines for the Python records layer.

Run after ``maturin develop`` with::

    python benchmarks/records_arrow.py --min-time 0.2 --repeat 7

Fixtures are prepared before timing. Results report throughput so changes can
be compared on the same interpreter, PyArrow build, and machine.
"""

from __future__ import annotations

import argparse
import gc
import platform
import statistics
import timeit
from dataclasses import dataclass
from datetime import date, datetime, time, timedelta, timezone
from decimal import Decimal
from typing import Any, Callable

import pyarrow as pa

from yggdryl import DataType, Field, Record


ROW_COUNT = 512
BATCH_SIZE = 64
DEEP_DEPTH = 12
DEEP_ROW_COUNT = 64
CAST_ROW_COUNT = 65_536


def _nested_struct(depth: int) -> pa.DataType:
    value: pa.DataType = pa.int64()
    for _ in range(depth):
        value = pa.struct([pa.field("value", value, nullable=False)])
    return value


def _nested_value(value: object, depth: int) -> object:
    for _ in range(depth):
        value = {"value": value}
    return value

SCHEMA = pa.schema(
    [
        pa.field("identifier", pa.uint64(), nullable=False),
        pa.field("active", pa.bool_(), nullable=False),
        pa.field("price", pa.decimal256(42, 8), nullable=False),
        pa.field("created", pa.timestamp("us", tz="UTC"), nullable=False),
        pa.field("settled", pa.date32(), nullable=False),
        pa.field("at", pa.time64("us"), nullable=False),
        pa.field("latency", pa.duration("us"), nullable=False),
        pa.field("payload", pa.binary(16), nullable=False),
        pa.field("labels", pa.large_list(pa.string()), nullable=False),
        pa.field(
            "metrics",
            pa.list_(
                pa.struct(
                    [
                        pa.field("name", pa.string(), nullable=False),
                        pa.field("value", pa.float64(), nullable=True),
                    ]
                )
            ),
            nullable=False,
        ),
        pa.field(
            "dimensions",
            pa.map_(pa.string(), pa.int16()),
            nullable=False,
        ),
        pa.field(
            "status",
            pa.dictionary(pa.int8(), pa.string()),
            nullable=False,
        ),
    ],
    metadata={b"benchmark": b"records-arrow"},
)
ROOT_FIELD = pa.field(
    "BenchmarkRow",
    pa.struct(list(SCHEMA)),
    nullable=False,
    metadata={b"benchmark": b"records-arrow"},
)

BenchmarkRow = Record.from_arrow_schema(
    SCHEMA,
    class_name="BenchmarkRow",
    module=__name__,
)
DEEP_SCHEMA = pa.schema(
    [pa.field("payload", _nested_struct(DEEP_DEPTH), nullable=False)]
)
DeepRow = Record.from_arrow_schema(
    DEEP_SCHEMA,
    class_name="DeepRow",
    module=__name__,
)

RAW_ROWS = tuple(
    {
        "identifier": str(index),
        "active": "true" if index % 2 else "false",
        "price": f"{index}.12500000",
        "created": "2026-08-15T10:30:00+00:00",
        "settled": "2026-08-15",
        "at": "10:30:00.123456",
        "latency": index / 1_000_000,
        "payload": index.to_bytes(16, "little"),
        "labels": ["orders", str(index % 8)],
        "metrics": [
            {"name": "price", "value": str(index + 0.25)},
            {"name": "missing", "value": None},
        ],
        "dimensions": {"shard": str(index % 16), "venue": "3"},
        "status": "ready" if index % 2 else "done",
    }
    for index in range(ROW_COUNT)
)
VALUES = tuple(BenchmarkRow.from_dicts(RAW_ROWS))
TYPED_ROWS = tuple(value.to_dict(safe=False) for value in VALUES)
BATCH = BenchmarkRow.into_arrow_record_batch(VALUES)
BATCHES = tuple(
    BenchmarkRow.into_arrow_record_batches(VALUES, batch_size=BATCH_SIZE)
)
TABLE = pa.Table.from_batches(BATCHES, schema=BenchmarkRow.into_arrow_schema())
TRANSPORT_BATCH = BATCH.replace_schema_metadata({b"transport": b"flight"})
DEEP_RAW_ROWS = tuple(
    {"payload": _nested_value(str(index), DEEP_DEPTH)}
    for index in range(DEEP_ROW_COUNT)
)
DEEP_VALUES = tuple(DeepRow.from_dicts(DEEP_RAW_ROWS))
DEEP_BATCH = DeepRow.into_arrow_record_batch(DEEP_VALUES)
SCALAR_DATA_TYPE = DataType("int32")
SCALAR_FIELD = Field("value", SCALAR_DATA_TYPE, nullable=False)
EXACT_SCALAR = pa.scalar(7, type=pa.int32())
WIDE_SCALAR = pa.scalar(7, type=pa.int64())
ScalarRow = Record.from_arrow_schema(
    pa.schema([pa.field("value", pa.int32(), nullable=False)]),
    class_name="ScalarRow",
    module=__name__,
)
SCALAR_MISMATCH_ROWS = (ScalarRow(WIDE_SCALAR),)
CAST_DATA_TYPE = DataType("int64")
CAST_SOURCE_ARRAY = pa.array(range(CAST_ROW_COUNT), type=pa.int32())
CAST_EXACT_ARRAY = pa.array(range(CAST_ROW_COUNT), type=pa.int64())
CAST_FIELD = Field(
    "cast",
    DataType.from_fields(
        [
            Field("id", "int64", nullable=False),
            Field("label", "utf8", nullable=True),
            Field("enabled", "boolean", nullable=False),
        ]
    ),
    nullable=False,
)
CAST_SOURCE_BATCH = pa.record_batch(
    [CAST_SOURCE_ARRAY, pa.array(["drop"] * CAST_ROW_COUNT)],
    names=["ID", "unused"],
)
CAST_EXACT_BATCH = CAST_FIELD.cast_arrow_batch(CAST_SOURCE_BATCH, safe=True)


def _dynamic_from_schema() -> type[Record]:
    return Record.from_arrow_schema(
        SCHEMA,
        class_name="ColdBenchmarkRow",
        module=__name__,
    )


def _dynamic_from_field() -> type[Record]:
    return Record.from_arrow_field(
        ROOT_FIELD,
        class_name="ColdBenchmarkRow",
        module=__name__,
    )


def _from_dicts_safe() -> object:
    return tuple(BenchmarkRow.from_dicts(RAW_ROWS))


def _from_dicts_shallow() -> object:
    return tuple(BenchmarkRow.from_dicts(TYPED_ROWS, safe=False))


def _batch_to_records_validated() -> object:
    return tuple(BenchmarkRow.from_arrow_record_batch(BATCH))


def _batch_with_transport_metadata() -> object:
    return tuple(BenchmarkRow.from_arrow_record_batch(TRANSPORT_BATCH))


def _batch_to_records_unvalidated() -> object:
    return tuple(
        BenchmarkRow.from_arrow_record_batch(BATCH, validate_schema=False)
    )


def _table_to_records() -> object:
    return tuple(BenchmarkRow.from_arrow_table(TABLE))


def _reader_to_records() -> object:
    reader = pa.RecordBatchReader.from_batches(
        BenchmarkRow.into_arrow_schema(),
        BATCHES,
    )
    return tuple(BenchmarkRow.from_arrow_record_batch_reader(reader))


class _StreamExporter:
    def __arrow_c_stream__(self, requested_schema: object = None) -> object:
        return TABLE.__arrow_c_stream__(requested_schema)


def _c_stream_to_records() -> object:
    return tuple(BenchmarkRow.from_arrow(_StreamExporter()))


def _records_to_batch() -> object:
    return BenchmarkRow.into_arrow_record_batch(VALUES)


def _records_to_batches() -> object:
    return tuple(
        BenchmarkRow.into_arrow_record_batches(VALUES, batch_size=BATCH_SIZE)
    )


def _records_to_table() -> object:
    return BenchmarkRow.into_arrow_table(VALUES)


def _records_to_reader() -> object:
    return BenchmarkRow.into_arrow_record_batch_reader(
        VALUES,
        batch_size=BATCH_SIZE,
    ).read_all()


def _records_to_first_batch() -> object:
    return next(
        BenchmarkRow.into_arrow_record_batches(VALUES, batch_size=BATCH_SIZE)
    )


def _deep_from_dicts() -> object:
    return tuple(DeepRow.from_dicts(DEEP_RAW_ROWS))


def _deep_batch_to_records() -> object:
    return tuple(DeepRow.from_arrow_record_batch(DEEP_BATCH))


def _deep_records_to_batch() -> object:
    return DeepRow.into_arrow_record_batch(DEEP_VALUES)


def _datatype_scalar_identity() -> object:
    return SCALAR_DATA_TYPE.arrow_scalar(EXACT_SCALAR)


def _field_scalar_identity() -> object:
    return SCALAR_FIELD.arrow_scalar(EXACT_SCALAR)


def _datatype_scalar_cast() -> object:
    return SCALAR_DATA_TYPE.arrow_scalar(WIDE_SCALAR)


def _field_scalar_cast() -> object:
    return SCALAR_FIELD.arrow_scalar(WIDE_SCALAR)


def _record_scalar_mismatch() -> object:
    return ScalarRow.into_arrow_record_batch(
        SCALAR_MISMATCH_ROWS, safe=False
    )


def _array_cast() -> object:
    return CAST_DATA_TYPE.cast_arrow_array(CAST_SOURCE_ARRAY, safe=True)


def _array_identity() -> object:
    return CAST_DATA_TYPE.cast_arrow_array(CAST_EXACT_ARRAY, safe=True)


def _batch_structural_cast() -> object:
    return CAST_FIELD.cast_arrow_batch(CAST_SOURCE_BATCH, safe=True)


def _batch_identity() -> object:
    return CAST_FIELD.cast_arrow_batch(CAST_EXACT_BATCH, safe=True)


@dataclass(frozen=True, slots=True)
class Benchmark:
    name: str
    operation: Callable[[], object]
    units: int
    unit_name: str


BENCHMARKS = (
    Benchmark("dynamic class from Schema", _dynamic_from_schema, 1, "class"),
    Benchmark("dynamic class from Field", _dynamic_from_field, 1, "class"),
    Benchmark(
        "cached into_arrow_schema",
        BenchmarkRow.into_arrow_schema,
        1,
        "projection",
    ),
    Benchmark("from_dicts safe", _from_dicts_safe, ROW_COUNT, "row"),
    Benchmark("from_dicts shallow", _from_dicts_shallow, ROW_COUNT, "row"),
    Benchmark(
        "batch import validated",
        _batch_to_records_validated,
        ROW_COUNT,
        "row",
    ),
    Benchmark(
        "batch metadata differs",
        _batch_with_transport_metadata,
        ROW_COUNT,
        "row",
    ),
    Benchmark(
        "batch import unvalidated",
        _batch_to_records_unvalidated,
        ROW_COUNT,
        "row",
    ),
    Benchmark("chunked Table import", _table_to_records, ROW_COUNT, "row"),
    Benchmark("RecordBatchReader import", _reader_to_records, ROW_COUNT, "row"),
    Benchmark("Arrow C stream import", _c_stream_to_records, ROW_COUNT, "row"),
    Benchmark("records to RecordBatch", _records_to_batch, ROW_COUNT, "row"),
    Benchmark("records to batches", _records_to_batches, ROW_COUNT, "row"),
    Benchmark("records to Table", _records_to_table, ROW_COUNT, "row"),
    Benchmark("records to reader", _records_to_reader, ROW_COUNT, "row"),
    Benchmark(
        "lazy output first batch",
        _records_to_first_batch,
        BATCH_SIZE,
        "row",
    ),
    Benchmark(
        f"depth-{DEEP_DEPTH} from_dicts",
        _deep_from_dicts,
        DEEP_ROW_COUNT,
        "row",
    ),
    Benchmark(
        f"depth-{DEEP_DEPTH} batch import",
        _deep_batch_to_records,
        DEEP_ROW_COUNT,
        "row",
    ),
    Benchmark(
        f"depth-{DEEP_DEPTH} batch output",
        _deep_records_to_batch,
        DEEP_ROW_COUNT,
        "row",
    ),
    Benchmark(
        "DataType scalar identity",
        _datatype_scalar_identity,
        1,
        "scalar",
    ),
    Benchmark("Field scalar identity", _field_scalar_identity, 1, "scalar"),
    Benchmark("DataType scalar cast", _datatype_scalar_cast, 1, "scalar"),
    Benchmark("Field scalar cast", _field_scalar_cast, 1, "scalar"),
    Benchmark("record scalar mismatch", _record_scalar_mismatch, 1, "row"),
    Benchmark("Arrow array native cast", _array_cast, CAST_ROW_COUNT, "row"),
    Benchmark(
        "Arrow array identity",
        _array_identity,
        CAST_ROW_COUNT,
        "row",
    ),
    Benchmark(
        "Arrow batch structural cast",
        _batch_structural_cast,
        CAST_ROW_COUNT,
        "row",
    ),
    Benchmark(
        "Arrow batch identity",
        _batch_identity,
        CAST_ROW_COUNT,
        "row",
    ),
    Benchmark("PyArrow to_pylist baseline", BATCH.to_pylist, ROW_COUNT, "row"),
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=7)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    assert VALUES[0].price == Decimal("0.12500000")
    assert VALUES[0].created == datetime(
        2026, 8, 15, 10, 30, tzinfo=timezone.utc
    )
    assert VALUES[0].settled == date(2026, 8, 15)
    assert VALUES[0].at == time(10, 30, 0, 123456)
    assert VALUES[1].latency == timedelta(microseconds=1)
    assert len(BATCHES) == ROW_COUNT // BATCH_SIZE
    assert len(DEEP_VALUES) == DEEP_ROW_COUNT

    print(
        f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
        f"{ROW_COUNT} rows, {len(SCHEMA)} columns, {len(BATCHES)} batches"
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


if __name__ == "__main__":
    main()
