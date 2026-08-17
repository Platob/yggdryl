"""Small reproducible baselines for the Python records layer.

Run after ``maturin develop`` with::

    python benchmarks/records.py --iterations 100000

The script uses only package runtime dependencies and no benchmark framework,
so benchmark tooling never enters the runtime dependency graph.
"""

from __future__ import annotations

import argparse
import gc
import statistics
import timeit
from collections.abc import Callable
from decimal import Decimal
from typing import Annotated

import pyarrow as pa

from yggdryl.records import record, schema_field, schema_fields


@record(frozen=True, slots=True)
class Leg:
    symbol: str
    quantity: int


@record(frozen=True, slots=True)
class Order:
    order_id: int
    active: bool
    legs: list[Leg]
    note: str | None = None


@record(frozen=True, slots=True)
class VariantValue:
    value: int | str


PAYLOAD = {
    "order_id": "42",
    "active": "true",
    "legs": [{"symbol": "ABC", "quantity": "10"}],
}
VALUE = Order.from_dict(PAYLOAD)
VARIANT_PAYLOAD = {"value": "42"}
VARIANT_VALUE = VariantValue.from_dict(VARIANT_PAYLOAD)
PRECISE_PRICE = pa.decimal128(18, 4)


def _cold_record_class() -> type[object]:
    @record(frozen=True, slots=True)
    class Quote:
        symbol: str
        bid: float
        ask: float
        venues: list[str]
        comment: str | None = None

    return Quote


def _cold_customized_record_class() -> type[object]:
    @record(frozen=True, slots=True)
    class PreciseQuote:
        price: Annotated[
            Decimal,
            ("arrow_type", PRECISE_PRICE),
            {"nullable": False, "metadata": {"unit": "EUR"}, "id": 7},
        ]

    return PreciseQuote


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:28} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=100_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    assert schema_field(Order) is schema_field(Order)
    assert schema_fields(Order) is schema_fields(Order)
    gc.disable()
    try:
        _measure("cached root field", lambda: schema_field(Order), args.iterations)
        _measure("cached child fields", lambda: schema_fields(Order), args.iterations)
        _measure("shallow from_dict", lambda: Order.from_dict(PAYLOAD, safe=False), args.iterations)
        _measure("safe nested from_dict", lambda: Order.from_dict(PAYLOAD), args.iterations)
        _measure("shallow to_dict", lambda: VALUE.to_dict(safe=False), args.iterations)
        _measure("safe nested to_dict", VALUE.to_dict, args.iterations)
        _measure(
            "safe variant from_dict",
            lambda: VariantValue.from_dict(VARIANT_PAYLOAD),
            args.iterations,
        )
        _measure("safe variant to_dict", VARIANT_VALUE.to_dict, args.iterations)
        _measure("cold class + schema", _cold_record_class, max(1, args.iterations // 100))
        _measure(
            "cold option schema",
            _cold_customized_record_class,
            max(1, args.iterations // 100),
        )
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
