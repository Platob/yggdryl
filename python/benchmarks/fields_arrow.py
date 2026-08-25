"""Field-class and PyArrow schema-boundary baselines.

Run after ``maturin develop`` with::

    python benchmarks/fields_arrow.py --iterations 10000
"""

from __future__ import annotations

import argparse
import gc
import statistics
import timeit
from collections.abc import Callable

import pyarrow as pa

from yggdryl import Field, field, json, scalar


ARROW_SCHEMA = pa.schema(
    [
        pa.field("identifier", pa.int16(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field(
            "payload",
            pa.struct(
                [
                    pa.field("price", pa.decimal128(18, 4), nullable=False),
                    pa.field("venues", pa.list_(pa.string())),
                ]
            ),
            nullable=False,
        ),
    ],
    metadata={"source": "benchmark"},
)
NATIVE_FIELD = Field.from_arrow_schema(ARROW_SCHEMA, name="quote")
GENERATED_CLASS = NATIVE_FIELD.into_dataclass(name="GeneratedQuote")


@scalar(frozen=True, slots=True)
class Payload:
    price: float
    venues: list[str] | None = None


@scalar(frozen=True, slots=True)
class Quote:
    identifier: int
    symbol: str
    payload: Payload


PAYLOAD = {
    "identifier": "42",
    "symbol": "ABC",
    "payload": {"price": "12.5", "venues": ["XNAS", "XPAR"]},
}
ENCODED_PAYLOAD = json.dumps(PAYLOAD)
VALUE = json.loads(ENCODED_PAYLOAD, cls=Quote)


def _import_arrow_schema() -> Field:
    return Field.from_arrow_schema(ARROW_SCHEMA, name="quote")


def _global_field() -> Field:
    return field(ARROW_SCHEMA, name="quote")


def _export_arrow_schema() -> pa.Schema:
    return NATIVE_FIELD.into_arrow_schema()


def _materialize_dataclass() -> type[object]:
    return NATIVE_FIELD.into_dataclass(name="GeneratedQuote")


def _cold_decorated_class() -> Field:
    @scalar(frozen=True, slots=True)
    class Reading:
        identifier: int
        symbol: str
        values: list[float]

    return Reading.field()


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:30} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    assert GENERATED_CLASS.field() is NATIVE_FIELD
    assert NATIVE_FIELD.into_arrow_schema() == ARROW_SCHEMA
    gc.disable()
    try:
        _measure("PyArrow schema import", _import_arrow_schema, args.iterations)
        _measure("global field", _global_field, args.iterations)
        _measure("PyArrow schema export", _export_arrow_schema, args.iterations)
        _measure(
            "cached static field",
            Quote.field,
            args.iterations,
        )
        _measure(
            "native to dataclass",
            _materialize_dataclass,
            max(1, args.iterations // 100),
        )
        _measure(
            "cold scalar + field",
            _cold_decorated_class,
            max(1, args.iterations // 100),
        )
        _measure(
            "codec materialize",
            lambda: json.loads(ENCODED_PAYLOAD, cls=Quote),
            args.iterations,
        )
        _measure("dataclass codec encode", lambda: json.dumps(VALUE), args.iterations)
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
