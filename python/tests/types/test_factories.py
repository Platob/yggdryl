from __future__ import annotations

import gc
import operator
from collections.abc import Iterator

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, enums, types


def test_every_native_datatype_variant_has_a_typed_field_factory() -> None:
    item = types.int8("item", nullable=False)
    entries = types.struct(
        "entries",
        [
            types.utf8("key", nullable=False),
            types.int64("value"),
        ],
        nullable=False,
    )
    run_ends = types.int16("run_ends", nullable=False)
    values = types.utf8("values")

    values_by_kind = {
        "null": types.null("value"),
        "boolean": types.boolean("value"),
        "int8": types.int8("value"),
        "int16": types.int16("value"),
        "int32": types.int32("value"),
        "int64": types.int64("value"),
        "uint8": types.uint8("value"),
        "uint16": types.uint16("value"),
        "uint32": types.uint32("value"),
        "uint64": types.uint64("value"),
        "float16": types.float16("value"),
        "float32": types.float32("value"),
        "float64": types.float64("value"),
        "datetime64": types.datetime64("value", "us", "Europe/Paris"),
        "date32": types.date32("value"),
        "date64": types.date64("value"),
        "time32": types.time32("value", "ms"),
        "time64": types.time64("value", "ns"),
        "duration32": types.duration32("value", "ms"),
        "duration64": types.duration64("value", "us"),
        "interval": types.interval("value", "month_day_nano"),
        "binary": types.binary("value"),
        "fixed_size_binary": types.fixed_size_binary("value", 16),
        "large_binary": types.large_binary("value"),
        "binary_view": types.binary_view("value"),
        "utf8": types.utf8("value"),
        "large_utf8": types.large_utf8("value"),
        "utf8_view": types.utf8_view("value"),
        "ascii": types.ascii("value"),
        "fixed_ascii": types.fixed_ascii("value", 4),
        "list": types.list("value", item),
        "list_view": types.list_view("value", item),
        "fixed_size_list": types.fixed_size_list("value", item, 3),
        "large_list": types.large_list("value", item),
        "large_list_view": types.large_list_view("value", item),
        "struct": types.struct("value", [item]),
        "union": types.union("value", [(3, item)], "dense"),
        "dictionary": types.dictionary("value", "int16", "utf8"),
        "decimal32": types.decimal32("value", 9, 2),
        "decimal64": types.decimal64("value", 18, 2),
        "decimal128": types.decimal128("value", 38, 2),
        "decimal256": types.decimal256("value", 76, 2),
        "map": types.map("value", entries, keys_sorted=True),
        "run_end_encoded": types.run_end_encoded(
            "value", run_ends, values
        ),
        "variant": types.variant("value"),
        "country": types.country("value"),
        "currency": types.currency("value"),
        "mic": types.mic("value"),
        "cfi": types.cfi("value"),
        "uuid": types.uuid("value"),
        "geometry": types.geometry("value"),
        "geography": types.geography("value", "OGC:CRS84", "vincenty"),
    }

    assert len(values_by_kind) == 52
    assert set(values_by_kind) == {
        value.dtype.id for value in values_by_kind.values()
    }
    # The factories cover every datatype Arrow has a layout for. `int128` and
    # `uint128` are the two identifiers `Scalar` stores and `DataType` cannot,
    # so no field builds them.
    assert set(values_by_kind) == set(enums.DATA_TYPE_IDS) - {"int128", "uint128"}
    assert all(type(value) is Field for value in values_by_kind.values())
    assert types.Int32Field is Field
    assert types.TypedField is Field


def test_nested_factories_preserve_exact_child_field_state() -> None:
    item = types.dictionary(
        "item",
        "int16",
        "utf8",
        nullable=False,
        metadata={"logical": "status"},
    )
    item.set_dictionary_options(42, True)
    projected_item = item.into_arrow()

    values = types.list("values", item, metadata={"owner": "events"})
    child = values.dtype[0]

    assert child.equals(item)
    assert child.dictionary_id == 42
    assert child.dictionary_is_ordered is True
    assert child.metadata["logical"] == "status"
    assert child.into_arrow().equals(projected_item, check_metadata=True)
    assert values.into_arrow().metadata == {b"owner": b"events"}


def test_dense_union_factory_is_a_typed_union_alias_with_native_ids() -> None:
    members = (
        types.int64("integer", nullable=False, metadata={"branch": "number"}),
        types.utf8("text", nullable=False),
    )

    value: types.DenseUnionField = types.dense_union(
        "payload",
        (member for member in members),
        nullable=False,
        metadata={"logical": "variant"},
    )
    arrow = value.into_arrow()

    assert type(value) is Field
    assert types.DenseUnionField is Field
    assert value.dtype.id == "union"
    assert tuple(value.dtype) == members
    assert arrow.type.mode == "dense"
    assert tuple(arrow.type.type_codes) == (0, 1)
    assert arrow.metadata == {b"logical": b"variant"}


def test_typed_factory_parameters_use_native_validation() -> None:
    assert types.decimal("small", 38).dtype.id == "decimal128"
    assert types.decimal("wide", 39).dtype.id == "decimal256"
    assert types.ascii("note").dtype == DataType("ascii")
    assert types.fixed_ascii("iso", 2).dtype == DataType.ascii(2)
    assert types.fixed_ascii("ccy", 3).dtype == DataType.ascii(3)
    # A fixed width past the packed integer is still storage, so it builds.
    assert types.fixed_ascii("isin", 64, nullable=False).dtype.ascii_width == 64
    assert types.currency("ccy", metadata={"code": "ISO 4217"}).metadata["code"] == (
        "ISO 4217"
    )
    assert types.time("coarse", "ms").dtype == DataType("time32(ms)")
    assert types.time("precise", "us").dtype == DataType("time64(us)")
    assert types.datetime64("event", "us", "Custom/Accepted").dtype.id == (
        "datetime64"
    )

    with pytest.raises(ValueError, match="temporal resolution"):
        types.time("clock", "day_time")
    with pytest.raises(ValueError, match="temporal resolution"):
        types.datetime64("event", "year_month")
    with pytest.raises(ValueError, match="interval layout"):
        types.interval("window", "us")
    with pytest.raises(ValueError, match="precision"):
        types.decimal32("amount", 10)
    with pytest.raises(ValueError, match="at least 1 byte"):
        types.fixed_ascii("narrow", 0)
    with pytest.raises(ValueError, match="run_ends"):
        types.run_end_encoded(
            "encoded",
            types.int16("run_ends", nullable=True),
            types.utf8("values"),
        )


def test_field_and_datatype_equality_can_ignore_recursive_metadata() -> None:
    left_child = types.int32(
        "id", nullable=False, metadata={"source": "left"}
    )
    right_child = types.int32(
        "id", nullable=False, metadata={"source": "right"}
    )
    left = types.struct("row", [left_child], metadata={"root": "left"})
    right = types.struct("row", [right_child], metadata={"root": "right"})

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
    mapping = types.map_of(
        "labels", "utf8", "int32", keys_sorted=True, nullable=False
    )
    arrow = mapping.into_arrow()

    assert arrow.type.equals(pa.map_(pa.string(), pa.int32(), keys_sorted=True))
    assert arrow.nullable is False


def test_dictionary_and_map_of_infer_python_and_pyarrow_type_inputs() -> None:
    dictionary = types.dictionary("status", int, str, nullable=False)
    mapping = types.map_of("labels", str, pa.int16(), nullable=False)

    assert str(dictionary.dtype) == "dictionary(int64,utf8)"
    assert mapping.dtype.id == "map"
    entries = mapping.dtype[0].dtype
    assert [field.dtype.id for field in entries] == ["utf8", "int16"]
