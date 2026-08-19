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
from typing import Annotated

from yggdryl import DataType, Field, MediaType, MimeType, fields

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


def _construct_wide_metadata() -> Field:
    return Field("payload", "utf8", metadata=WIDE_UPDATE)


def _update_wide_metadata() -> Field:
    field = Field("payload", "utf8")
    field.update(WIDE_UPDATE)
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


def _default_python_record() -> object:
    return DEFAULT_STRUCT.default_pyvalue()


def _default_arrow_scalar() -> object:
    return DEFAULT_FIELD.default_arrow_scalar()


def _spark_compatibility() -> DataType:
    return DEFAULT_STRUCT.to_scheme_compat("spark")


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
    return PROTOCOL_FIELD["iceberg:doc"]


def _protocol_view_items() -> object:
    return list(PROTOCOL_FIELD.iceberg.items())


def _write_through_protocol_view() -> None:
    PROTOCOL_FIELD.iceberg["doc"] = "closing price"


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
        _measure("default Python record", _default_python_record, args.iterations)
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
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
