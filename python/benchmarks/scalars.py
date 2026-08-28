"""Native Scalar and Arrow C Data baselines.

Run after ``maturin develop`` with::

    python benchmarks/scalars.py --iterations 10000
"""

from __future__ import annotations

import argparse
import copy
import gc
import pickle
import statistics
import timeit
from collections.abc import Callable
from dataclasses import dataclass

import pyarrow as pa

from yggdryl import Expression, Field, Url, Scalar
from yggdryl.iceberg import IcebergOptions, PartitionSpec, ScanPlan


@dataclass
class InferredRow:
    id: int
    symbol: str


PYTHON_VALUE = {"id": 42, "symbol": "AAPL", "levels": [1.0, 2.0, 3.0]}
NATIVE_VALUE = Scalar.from_py(PYTHON_VALUE)
NATIVE_LEGS = NATIVE_VALUE["levels"]
NATIVE_TEMPORAL = Scalar.datetime64(1_700_000_000_000_000, "us", "UTC")
NATIVE_DECIMAL = Scalar.d256("1234567890123456789012345678901234567890", 6)
NATIVE_INTEGER = Scalar.from_py(84)
NATIVE_DIVISOR = Scalar.from_py(2)
NATIVE_ENUM = Scalar.from_enum("io_mode", "append")
PRICE_EXPRESSION = Expression.column("price")
ARROW_SCALAR = pa.scalar(12.5, pa.float32())
NATIVE_SCALAR = Scalar.from_arrow_scalar(ARROW_SCALAR)
ARROW_ARRAY = pa.array(range(4096), type=pa.int32())
NATIVE_ARRAY = Scalar.from_arrow_array(ARROW_ARRAY)
INFERRED_ROWS = Scalar.from_py([InferredRow(1, "AAPL")])
ARROW_BATCH = pa.record_batch(
    [ARROW_ARRAY, pa.array(["AAPL"] * len(ARROW_ARRAY))], names=["id", "symbol"]
)
ROOT = Field.from_arrow_schema(ARROW_BATCH.schema)
NATIVE_ROWS = Scalar.from_arrow_batch(ARROW_BATCH)
ARROW_TABLE = pa.Table.from_batches([ARROW_BATCH])
URL_TEXT = "https://example.com/archive/data.json"
URL_VALUE = Url(URL_TEXT)
ICEBERG_SPEC = PartitionSpec.unpartitioned()
ICEBERG_OPTIONS = IcebergOptions(commit_retries=4, data_mime_type="parquet")
SCAN_REPORT = ScanPlan._from_pickle(4096, 8, 24, 2, 6)


def _hash_fresh_url() -> int:
    """Include the one-time hash-lock transition, not only repeated hashing."""
    return hash(Url(URL_TEXT))


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    nanoseconds = statistics.median(samples) * 1_000_000_000 / iterations
    print(f"{name:31} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    arguments = parser.parse_args()
    if arguments.iterations < 1:
        parser.error("--iterations must be positive")

    small = arguments.iterations
    bulk = max(1, small // 100)
    gc.disable()
    try:
        for name, operation, iterations in (
            ("from Python", lambda: Scalar.from_py(PYTHON_VALUE), small),
            ("into Python", NATIVE_VALUE.as_py, small),
            ("stable hash", NATIVE_VALUE.stable_hash, small),
            ("enum construction", lambda: Scalar.from_enum("io_mode", "append"), small),
            ("enum kind", lambda: NATIVE_ENUM.enum_kind, small),
            ("enum value", lambda: NATIVE_ENUM.enum_value, small),
            ("enum ordinal", lambda: NATIVE_ENUM.enum_ordinal, small),
            ("Python hash", lambda: hash(NATIVE_VALUE), small),
            ("exact Scalar repr", lambda: repr(NATIVE_VALUE), small),
            ("Scalar pickle dump", lambda: pickle.dumps(NATIVE_VALUE), small),
            (
                "Scalar pickle round trip",
                lambda: pickle.loads(pickle.dumps(NATIVE_VALUE)),
                small,
            ),
            ("URL stable hash", URL_VALUE.stable_hash, small),
            ("URL hash lock", _hash_fresh_url, small),
            ("URL unlocked copy", lambda: copy.copy(URL_VALUE), small),
            ("Iceberg spec stable hash", ICEBERG_SPEC.stable_hash, small),
            ("Iceberg options hash lock", lambda: hash(copy.copy(ICEBERG_OPTIONS)), small),
            ("scan report stable hash", SCAN_REPORT.stable_hash, small),
            ("JSON bytes", NATIVE_VALUE.as_json_bytes, small),
            ("JSON UTF-8", NATIVE_VALUE.as_json_utf8, small),
            ("mapping get", lambda: NATIVE_VALUE.get("symbol"), small),
            ("dotted path", lambda: NATIVE_VALUE.path("levels.1"), small),
            ("persistent mapping set", lambda: NATIVE_VALUE.set("id", 43), small),
            ("native child iteration", lambda: tuple(NATIVE_LEGS), small),
            ("mapping item iteration", lambda: tuple(NATIVE_VALUE.items()), small),
            ("temporal count", lambda: NATIVE_TEMPORAL.count, small),
            ("temporal unit", lambda: NATIVE_TEMPORAL.unit, small),
            ("temporal zone", lambda: NATIVE_TEMPORAL.zone, small),
            ("D256 unscaled", lambda: NATIVE_DECIMAL.unscaled, small),
            ("decimal scale", lambda: NATIVE_DECIMAL.scale, small),
            ("checked add", lambda: NATIVE_INTEGER.add(NATIVE_DIVISOR), small),
            ("checked subtract", lambda: NATIVE_INTEGER.subtract(NATIVE_DIVISOR), small),
            ("checked multiply", lambda: NATIVE_INTEGER.multiply(NATIVE_DIVISOR), small),
            ("checked divide", lambda: NATIVE_INTEGER.divide(NATIVE_DIVISOR), small),
            ("checked remainder", lambda: NATIVE_INTEGER.remainder(NATIVE_DIVISOR), small),
            ("checked negate", NATIVE_INTEGER.negate, small),
            ("checked absolute", NATIVE_INTEGER.absolute, small),
            ("operator add", lambda: NATIVE_INTEGER + 2, small),
            ("reflected add", lambda: 2 + NATIVE_INTEGER, small),
            ("expression named add", lambda: PRICE_EXPRESSION.add(2), small),
            ("expression operator add", lambda: PRICE_EXPRESSION + 2, small),
            ("expression reflected sub", lambda: 2 - PRICE_EXPRESSION, small),
            ("infer scalar Field", NATIVE_SCALAR.into_field, small),
            ("infer array Field", NATIVE_ARRAY.into_array_field, small),
            ("infer struct Field", INFERRED_ROWS.into_struct_field, small),
            ("from Arrow scalar", lambda: Scalar.from_arrow_scalar(ARROW_SCALAR), small),
            ("into Arrow scalar", NATIVE_SCALAR.into_arrow_scalar, small),
            ("from Arrow array (4096)", lambda: Scalar.from_arrow_array(ARROW_ARRAY), bulk),
            ("into Arrow array (4096)", NATIVE_ARRAY.into_arrow_array, bulk),
            ("from Arrow batch (4096)", lambda: Scalar.from_arrow_batch(ARROW_BATCH), bulk),
            ("into Arrow batch (4096)", lambda: NATIVE_ROWS.into_arrow_batch(ROOT), bulk),
            ("from Arrow table (4096)", lambda: Scalar.from_arrow_table(ARROW_TABLE), bulk),
            ("into Arrow table (4096)", lambda: NATIVE_ROWS.into_arrow_table(ROOT), bulk),
        ):
            _measure(name, operation, iterations)
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
