from __future__ import annotations

import gc
import operator
from collections.abc import Iterator

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, enums, fields


def test_every_native_datatype_variant_has_a_typed_field_factory() -> None:
    item = fields.int8("item", nullable=False)
    entries = fields.struct(
        "entries",
        [
            fields.utf8("key", nullable=False),
            fields.int64("value"),
        ],
        nullable=False,
    )
    run_ends = fields.int16("run_ends", nullable=False)
    values = fields.utf8("values")

    values_by_kind = {
        "null": fields.null("value"),
        "boolean": fields.boolean("value"),
        "int8": fields.int8("value"),
        "int16": fields.int16("value"),
        "int32": fields.int32("value"),
        "int64": fields.int64("value"),
        "uint8": fields.uint8("value"),
        "uint16": fields.uint16("value"),
        "uint32": fields.uint32("value"),
        "uint64": fields.uint64("value"),
        "float16": fields.float16("value"),
        "float32": fields.float32("value"),
        "float64": fields.float64("value"),
        "timestamp": fields.timestamp("value", "us", "Europe/Paris"),
        "date32": fields.date32("value"),
        "date64": fields.date64("value"),
        "time32": fields.time32("value", "ms"),
        "time64": fields.time64("value", "ns"),
        "duration32": fields.duration32("value", "ms"),
        "duration64": fields.duration64("value", "us"),
        "interval": fields.interval("value", "month_day_nano"),
        "binary": fields.binary("value"),
        "fixed_size_binary": fields.fixed_size_binary("value", 16),
        "large_binary": fields.large_binary("value"),
        "binary_view": fields.binary_view("value"),
        "utf8": fields.utf8("value"),
        "large_utf8": fields.large_utf8("value"),
        "utf8_view": fields.utf8_view("value"),
        "ascii16": fields.ascii16("value"),
        "ascii24": fields.ascii24("value"),
        "ascii32": fields.ascii32("value"),
        "ascii64": fields.ascii64("value"),
        "ascii96": fields.ascii96("value"),
        "ascii128": fields.ascii128("value"),
        "list": fields.list("value", item),
        "list_view": fields.list_view("value", item),
        "fixed_size_list": fields.fixed_size_list("value", item, 3),
        "large_list": fields.large_list("value", item),
        "large_list_view": fields.large_list_view("value", item),
        "struct": fields.struct("value", [item]),
        "union": fields.union("value", [(3, item)], "dense"),
        "dictionary": fields.dictionary("value", "int16", "utf8"),
        "decimal32": fields.decimal32("value", 9, 2),
        "decimal64": fields.decimal64("value", 18, 2),
        "decimal128": fields.decimal128("value", 38, 2),
        "decimal256": fields.decimal256("value", 76, 2),
        "map": fields.map("value", entries, keys_sorted=True),
        "run_end_encoded": fields.run_end_encoded(
            "value", run_ends, values
        ),
        "variant": fields.variant("value"),
        "country": fields.country("value"),
        "currency": fields.currency("value"),
        "mic": fields.mic("value"),
        "cfi": fields.cfi("value"),
        "guid": fields.guid("value"),
        "geometry": fields.geometry("value"),
        "geography": fields.geography("value", "OGC:CRS84", "vincenty"),
    }

    assert len(values_by_kind) == 56
    assert set(values_by_kind) == {
        value.dtype.id for value in values_by_kind.values()
    }
    # The factories cover the whole vocabulary, not a chosen part of it.
    assert set(values_by_kind) == set(enums.DATA_TYPE_IDS)
    assert all(type(value) is Field for value in values_by_kind.values())
    assert fields.Int32Field is Field
    assert fields.TypedField is Field


def test_nested_factories_preserve_exact_child_field_state() -> None:
    item = fields.dictionary(
        "item",
        "int16",
        "utf8",
        nullable=False,
        metadata={"logical": "status"},
    )
    item.set_dictionary_options(42, True)
    projected_item = item.into_arrow()

    values = fields.list("values", item, metadata={"owner": "events"})
    child = values.dtype[0]

    assert child.equals(item)
    assert child.dictionary_id == 42
    assert child.dictionary_is_ordered is True
    assert child.metadata["logical"] == "status"
    assert child.into_arrow().equals(projected_item, check_metadata=True)
    assert values.into_arrow().metadata == {b"owner": b"events"}


def test_dense_union_factory_is_a_typed_union_alias_with_native_ids() -> None:
    members = (
        fields.int64("integer", nullable=False, metadata={"branch": "number"}),
        fields.utf8("text", nullable=False),
    )

    value: fields.DenseUnionField = fields.dense_union(
        "payload",
        (member for member in members),
        nullable=False,
        metadata={"logical": "variant"},
    )
    arrow = value.into_arrow()

    assert type(value) is Field
    assert fields.DenseUnionField is Field
    assert value.dtype.id == "union"
    assert tuple(value.dtype) == members
    assert arrow.type.mode == "dense"
    assert tuple(arrow.type.type_codes) == (0, 1)
    assert arrow.metadata == {b"logical": b"variant"}


def test_typed_factory_parameters_use_native_validation() -> None:
    assert fields.decimal("small", 38).dtype.id == "decimal128"
    assert fields.decimal("wide", 39).dtype.id == "decimal256"
    assert fields.ascii("iso", 2).dtype == DataType("ascii16")
    assert fields.ascii("ccy", 3).dtype == DataType("ascii24")
    assert fields.ascii("cfi", 6).dtype == DataType("ascii64")
    assert fields.ascii("isin", 12, nullable=False).dtype.id == "ascii96"
    assert fields.ascii32("ccy", metadata={"code": "ISO 4217"}).metadata["code"] == (
        "ISO 4217"
    )
    assert fields.time("coarse", "ms").dtype == DataType("time32(ms)")
    assert fields.time("precise", "us").dtype == DataType("time64(us)")
    assert fields.timestamp("event", "us", "Custom/Accepted").dtype.id == (
        "timestamp"
    )

    with pytest.raises(ValueError, match="temporal resolution"):
        fields.time("clock", "day_time")
    with pytest.raises(ValueError, match="temporal resolution"):
        fields.timestamp("event", "year_month")
    with pytest.raises(ValueError, match="interval layout"):
        fields.interval("window", "us")
    with pytest.raises(ValueError, match="precision"):
        fields.decimal32("amount", 10)
    with pytest.raises(ValueError, match="from 1 to 16 bytes"):
        fields.ascii("wide", 17)
    with pytest.raises(ValueError, match="run_ends"):
        fields.run_end_encoded(
            "encoded",
            fields.int16("run_ends", nullable=True),
            fields.utf8("values"),
        )


def test_field_and_datatype_equality_can_ignore_recursive_metadata() -> None:
    left_child = fields.int32(
        "id", nullable=False, metadata={"source": "left"}
    )
    right_child = fields.int32(
        "id", nullable=False, metadata={"source": "right"}
    )
    left = fields.struct("row", [left_child], metadata={"root": "left"})
    right = fields.struct("row", [right_child], metadata={"root": "right"})

    assert not left.equals(right)
    assert left.equals(right, with_metadata=False)
    assert not left.dtype.equals(right.dtype)
    assert left.dtype.equals(right.dtype, with_metadata=False)
    assert left.show_diff(left) == "✓ equal"
    assert left.dtype.show_diff(left.dtype) == "✓ equal"

    differences = left.show_diffs(right)
    assert isinstance(differences, Iterator)
    assert iter(differences) is differences
    lines = list(differences)
    assert any("metadata" in line and "≠" in line for line in lines)
    assert all("\x1b" not in line for line in lines)
    assert list(left.show_diffs(right, with_metadata=False)) == []


def test_differences_report_physical_layout_after_metadata_is_ignored() -> None:
    left = Field("value", DataType("int32"), nullable=False)
    right = Field("value", DataType("int64"), nullable=True)

    assert not left.equals(right, with_metadata=False)
    output = left.show_diff(right, with_metadata=False)
    assert "$.nullable" in output
    assert "$.dtype" in output
    assert "≠" in output
    assert output == "\n".join(left.show_diffs(right, with_metadata=False))


def test_wide_difference_iterator_is_lazy_and_outlives_field_sources() -> None:
    left = Field(
        "root",
        DataType.from_fields(
            Field(f"left_{index:04d}", "int32") for index in range(1024)
        ),
    )
    right = Field(
        "root",
        DataType.from_fields(
            Field(f"right_{index:04d}", "int32") for index in range(1024)
        ),
    )

    differences = left.show_diffs(right)
    assert not hasattr(differences, "__length_hint__")
    assert operator.length_hint(differences) == 0
    first = next(differences)
    assert first.startswith("≠ $.dtype.fields[0].name:")

    del left, right
    gc.collect()
    remaining = list(differences)

    assert len(remaining) == 1023
    assert remaining[-1].startswith("≠ $.dtype.fields[1023].name:")


def test_datatype_difference_iterator_outlives_source_wrappers() -> None:
    left = DataType.from_fields(
        [Field("value", "int64", metadata={"source": "left"})]
    )
    right = DataType.from_fields(
        [Field("value", "int64", metadata={"source": "right"})]
    )
    differences = left.show_diffs(right)

    del left, right
    gc.collect()

    assert list(differences) == [
        '≠ $.fields[0].metadata["source"]: "left" → "right"'
    ]


def test_map_factory_projects_exact_arrow_layout() -> None:
    mapping = fields.map_of(
        "labels", "utf8", "int32", keys_sorted=True, nullable=False
    )
    arrow = mapping.into_arrow()

    assert arrow.type.equals(pa.map_(pa.string(), pa.int32(), keys_sorted=True))
    assert arrow.nullable is False


def test_dictionary_and_map_of_infer_python_and_pyarrow_type_inputs() -> None:
    dictionary = fields.dictionary("status", int, str, nullable=False)
    mapping = fields.map_of("labels", str, pa.int16(), nullable=False)

    assert str(dictionary.dtype) == "dictionary(int64,utf8)"
    assert mapping.dtype.id == "map"
    entries = mapping.dtype[0].dtype
    assert [field.dtype.id for field in entries] == ["utf8", "int16"]
