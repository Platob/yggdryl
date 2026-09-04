"""Allocation-sensitive baselines for Python datatype and field boundaries.

Run after ``maturin develop`` with::

    python benchmarks/fields.py --iterations 10000

The wide metadata cases guard the bulk accumulator against accidental
quadratic duplicate handling. The inference cases exercise native nested
builders without projecting through PyArrow.
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

from yggdryl import (
    AsciiEnum,
    DataType,
    Field,
    MediaType,
    MimeType,
    scalar,
    fields,
    field,
    json,
)

WIDE_METADATA = tuple(
    (f"key_{index:04d}", str(index)) for index in range(1_024)
)
WIDE_UPDATE = (*WIDE_METADATA, ("key_0000", "updated"))
WIDE_DIFF_LEFT = Field(
    "root",
    DataType.from_fields(
        Field(f"left_{index:04d}", "int32") for index in range(1_024)
    ),
)
WIDE_DIFF_RIGHT = Field(
    "root",
    DataType.from_fields(
        Field(f"right_{index:04d}", "int32") for index in range(1_024)
    ),
)
PROTOCOL_FIELD = Field(
    "price",
    "float64",
    nullable=False,
    metadata={
        "iceberg:doc": "closing price",
        "iceberg:field-id": "7",
        "iceberg:schema-id": "3",
        "postgres:column": "close",
        "postgres:type": "numeric(18,6)",
        "venue": "XPAR",
    },
)
# A view held across reads is the shape a caller who reads several properties
# of one protocol ends up with; creating one per read is measured separately.
HELD_PROTOCOL_VIEW = PROTOCOL_FIELD.iceberg
KNOWN_MIME = "application/json"
CUSTOM_MIME = "application/vnd.benchmark+json"
COMPOUND_MEDIA = "text/csv;encodings=application/gzip,application/zstd"
CONTENT_TYPE = "text/csv; charset=utf-8"
CONTENT_ENCODING = "gzip, zstd"
DEFAULT_STRUCT = DataType.from_fields(
    (
        Field("count", "uint32", nullable=False),
        Field("label", "utf8", nullable=False),
    )
)
DEFAULT_FIELD = Field("optional", "decimal128(18,4)")
VARIANT_MEMBERS = (
    Field("integer", "int64", nullable=False),
    Field("text", "utf8", nullable=False),
)
PRECISE_PRICE = pa.decimal128(18, 4)


@scalar(frozen=True, slots=True)
class Leg:
    symbol: str
    quantity: int


@scalar(frozen=True, slots=True)
class Order:
    order_id: int
    active: bool
    legs: list[Leg]
    note: str | None = None


@scalar(frozen=True, slots=True)
class VariantValue:
    value: int | str


CLASS_PAYLOAD = {
    "order_id": "42",
    "active": "true",
    "legs": [{"symbol": "ABC", "quantity": "10"}],
}
CLASS_PAYLOAD_BYTES = json.dumps(CLASS_PAYLOAD)
CLASS_VALUE = json.loads(CLASS_PAYLOAD_BYTES, cls=Order)
VARIANT_PAYLOAD = {"value": "42"}
VARIANT_PAYLOAD_BYTES = json.dumps(VARIANT_PAYLOAD)
VARIANT_VALUE = json.loads(VARIANT_PAYLOAD_BYTES, cls=VariantValue)


def _construct_wide_metadata() -> Field:
    return Field("payload", "utf8", metadata=WIDE_UPDATE)


def _update_wide_metadata() -> Field:
    field = Field("payload", "utf8")
    field.metadata.update(WIDE_UPDATE)
    return field


def _infer_nested_datatype() -> DataType:
    return DataType.from_pyhint(
        dict[str, list[Annotated[int | None, {"unit": "ticks"}]]]
    )


def _build_nested_typed_field() -> Field:
    return fields.map_of("counts", str, int, nullable=False)


def _build_generic_time_datatype() -> DataType:
    return DataType.time("us")


def _build_generic_time_field() -> Field:
    return fields.time("at", "us", nullable=False)


def _build_ascii_datatype() -> DataType:
    return DataType.ascii(3)


def _build_ascii_field() -> Field:
    return fields.fixed_ascii("ccy", 4, nullable=False)


def _build_code_datatype() -> DataType:
    return DataType("currency")


def _build_code_field() -> Field:
    return fields.currency("ccy", nullable=False)


def _build_variant_datatype() -> DataType:
    return DataType.variant(VARIANT_MEMBERS)


def _build_variant_field() -> Field:
    return fields.dense_union("payload", VARIANT_MEMBERS, nullable=False)


def _infer_variant_datatype() -> DataType:
    return DataType.from_pyhint(int | str)


def _create_wide_difference_iterator() -> object:
    return WIDE_DIFF_LEFT.show_diffs(WIDE_DIFF_RIGHT)


def _first_wide_difference() -> str:
    return next(WIDE_DIFF_LEFT.show_diffs(WIDE_DIFF_RIGHT))


def _cached_default_hint() -> object:
    return DEFAULT_STRUCT.default_pyhint()


def _default_python_value() -> object:
    return DEFAULT_STRUCT.default_pyvalue()


def _global_field() -> Field:
    return field(Order)


def _renamed_field() -> Field:
    return field(Order, name="order")


def _cold_field_class() -> object:
    @scalar(frozen=True, slots=True)
    class Quote:
        symbol: str
        bid: float
        ask: float
        venues: list[str]
        comment: str | None = None

    return Quote.field()


def _cold_customized_field_class() -> object:
    @scalar(frozen=True, slots=True)
    class PreciseQuote:
        price: Annotated[
            Decimal,
            ("arrow_type", PRECISE_PRICE),
            {"nullable": False, "metadata": {"unit": "EUR"}, "id": 7},
        ]

    return PreciseQuote.field()


def _default_arrow_scalar() -> object:
    return DEFAULT_FIELD.default_arrow_scalar()


def _spark_compatibility() -> DataType:
    return DEFAULT_STRUCT.into_scheme_compat("spark")


# The protocol cases measure the boundary the live view adds: creating one is a
# scheme clone plus a reference to the field, and each read then crosses into
# the core exactly as ``get_property`` does, with the key assembled by the view
# instead of by the caller.
def _create_protocol_view() -> object:
    return PROTOCOL_FIELD.iceberg


def _read_through_protocol_view() -> object:
    return PROTOCOL_FIELD.iceberg["doc"]


def _read_through_held_protocol_view() -> object:
    return HELD_PROTOCOL_VIEW["doc"]


def _read_through_get_property() -> object:
    return PROTOCOL_FIELD.get_property("iceberg", "doc")


def _read_through_metadata_key() -> object:
    return PROTOCOL_FIELD.metadata["iceberg:doc"]


def _protocol_view_items() -> object:
    return list(PROTOCOL_FIELD.iceberg.items())


def _write_through_protocol_view() -> None:
    PROTOCOL_FIELD.iceberg["doc"] = "closing price"


CURRENCY = DataType("currency")
CURRENCIES = AsciiEnum.from_logical_name("currency")


def _ascii_prebuilt_vocabulary() -> object:
    # What a schema pays once when it declares a currency column.
    return AsciiEnum.from_logical_name("currency")


def _ascii_vocabulary_members() -> object:
    # The packed code of every declared value, which is what a reader of the
    # schema computes once.
    return CURRENCIES.into_members(CURRENCY)


def _ascii_vocabulary_intenum() -> object:
    return CURRENCIES.into_intenum(CURRENCY)


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    median = statistics.median(samples)
    nanoseconds = median * 1_000_000_000 / iterations
    print(f"{name:32} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")

    gc.disable()
    try:
        wide_iterations = max(1, args.iterations // 100)
        _measure(
            "wide metadata construction",
            _construct_wide_metadata,
            wide_iterations,
        )
        _measure("wide metadata update", _update_wide_metadata, wide_iterations)
        _measure("native nested inference", _infer_nested_datatype, args.iterations)
        _measure("native typed map", _build_nested_typed_field, args.iterations)
        _measure("generic time datatype", _build_generic_time_datatype, args.iterations)
        _measure("generic time field", _build_generic_time_field, args.iterations)
        _measure("ascii width datatype", _build_ascii_datatype, args.iterations)
        _measure("ascii width field", _build_ascii_field, args.iterations)
        _measure("code datatype", _build_code_datatype, args.iterations)
        _measure("code field", _build_code_field, args.iterations)
        _measure("native variant datatype", _build_variant_datatype, args.iterations)
        _measure("native variant field", _build_variant_field, args.iterations)
        _measure("inferred variant datatype", _infer_variant_datatype, args.iterations)
        _measure(
            "wide diff iterator creation",
            _create_wide_difference_iterator,
            args.iterations,
        )
        _measure("wide diff first line", _first_wide_difference, args.iterations)
        _measure("cached default hint", _cached_default_hint, args.iterations)
        _measure("default Python value", _default_python_value, args.iterations)
        _measure(
            "cached static field",
            Order.field,
            args.iterations,
        )
        _measure("global field", _global_field, args.iterations)
        _measure("renamed field", _renamed_field, args.iterations)
        _measure(
            "cached class child",
            lambda: Order.field().dtype["order_id"],
            args.iterations,
        )
        _measure(
            "shallow codec materialize",
            lambda: json.loads(CLASS_PAYLOAD_BYTES, cls=Order, safe=False),
            args.iterations,
        )
        _measure(
            "safe codec materialize",
            lambda: json.loads(CLASS_PAYLOAD_BYTES, cls=Order),
            args.iterations,
        )
        _measure(
            "dataclass codec encode",
            lambda: json.dumps(CLASS_VALUE),
            args.iterations,
        )
        _measure(
            "variant materialize",
            lambda: json.loads(VARIANT_PAYLOAD_BYTES, cls=VariantValue),
            args.iterations,
        )
        _measure(
            "variant encode",
            lambda: json.dumps(VARIANT_VALUE),
            args.iterations,
        )
        _measure(
            "cold scalar + field",
            _cold_field_class,
            max(1, args.iterations // 100),
        )
        _measure(
            "cold option field",
            _cold_customized_field_class,
            max(1, args.iterations // 100),
        )
        _measure("default Arrow scalar", _default_arrow_scalar, args.iterations)
        _measure("Spark compatibility", _spark_compatibility, args.iterations)
        _measure("protocol view creation", _create_protocol_view, args.iterations)
        _measure("protocol view read", _read_through_protocol_view, args.iterations)
        _measure(
            "protocol held view read",
            _read_through_held_protocol_view,
            args.iterations,
        )
        _measure("protocol get_property", _read_through_get_property, args.iterations)
        _measure("protocol metadata key", _read_through_metadata_key, args.iterations)
        _measure("protocol view items", _protocol_view_items, args.iterations)
        _measure("protocol view write", _write_through_protocol_view, args.iterations)
        _measure(
            "MIME known parse",
            lambda: MimeType.from_str(KNOWN_MIME),
            args.iterations,
        )
        _measure(
            "MIME custom parse",
            lambda: MimeType.from_str(CUSTOM_MIME),
            args.iterations,
        )
        _measure(
            "media compound parse",
            lambda: MediaType.from_str(COMPOUND_MEDIA),
            args.iterations,
        )
        _measure(
            "media header inference",
            lambda: MediaType.from_content_headers(CONTENT_TYPE, CONTENT_ENCODING),
            args.iterations,
        )
        _measure(
            "ASCII vocabulary prebuilt",
            _ascii_prebuilt_vocabulary,
            max(1, args.iterations // 100),
        )
        _measure(
            "ASCII vocabulary members",
            _ascii_vocabulary_members,
            max(1, args.iterations // 100),
        )
        _measure(
            "ASCII vocabulary IntEnum",
            _ascii_vocabulary_intenum,
            max(1, args.iterations // 1_000),
        )
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
