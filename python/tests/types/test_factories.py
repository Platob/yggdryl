from __future__ import annotations

import gc
import operator
from collections.abc import Iterator

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, enums


def test_every_native_datatype_variant_has_a_typed_field_factory() -> None:
    item = yggdryl.int8("item", nullable=False)
    entries = yggdryl.struct(
        "entries",
        [
            yggdryl.utf8("key", nullable=False),
            yggdryl.int64("value"),
        ],
        nullable=False,
    )
    run_ends = yggdryl.int16("run_ends", nullable=False)
    values = yggdryl.utf8("values")

    values_by_kind = {
        "null": yggdryl.null("value"),
        "boolean": yggdryl.boolean("value"),
        "int8": yggdryl.int8("value"),
        "int16": yggdryl.int16("value"),
        "int32": yggdryl.int32("value"),
        "int64": yggdryl.int64("value"),
        "uint8": yggdryl.uint8("value"),
        "uint16": yggdryl.uint16("value"),
        "uint32": yggdryl.uint32("value"),
        "uint64": yggdryl.uint64("value"),
        "float16": yggdryl.float16("value"),
        "float32": yggdryl.float32("value"),
        "float64": yggdryl.float64("value"),
        "datetime64": yggdryl.datetime64("value", "us", "Europe/Paris"),
        "date32": yggdryl.date32("value"),
        "date64": yggdryl.date64("value"),
        "time32": yggdryl.time32("value", "ms"),
        "time64": yggdryl.time64("value", "ns"),
        "duration32": yggdryl.duration32("value", "ms"),
        "duration64": yggdryl.duration64("value", "us"),
        "interval": yggdryl.interval("value", "month_day_nano"),
        "binary": yggdryl.binary("value"),
        "fixed_binary": yggdryl.fixed_size_binary("value", 16),
        "sized_binary": yggdryl.sized_binary("value", 16),
        "large_binary": yggdryl.large_binary("value"),
        "binary_view": yggdryl.binary_view("value"),
        "large_binary_view": yggdryl.large_binary_view("value"),
        "utf8": yggdryl.utf8("value"),
        "large_utf8": yggdryl.large_utf8("value"),
        "utf8_view": yggdryl.utf8_view("value"),
        "large_utf8_view": yggdryl.large_utf8_view("value"),
        "fixed_utf8": yggdryl.fixed_utf8("value", 8),
        "sized_utf8": yggdryl.sized_utf8("value", 32),
        "ascii": yggdryl.ascii("value"),
        "large_ascii": yggdryl.large_ascii("value"),
        "ascii_view": yggdryl.ascii_view("value"),
        "large_ascii_view": yggdryl.large_ascii_view("value"),
        "fixed_ascii": yggdryl.fixed_ascii("value", 4),
        "sized_ascii": yggdryl.sized_ascii("value", 4),
        "cp1252": yggdryl.cp1252("value"),
        "large_cp1252": yggdryl.large_cp1252("value"),
        "cp1252_view": yggdryl.cp1252_view("value"),
        "large_cp1252_view": yggdryl.large_cp1252_view("value"),
        "fixed_cp1252": yggdryl.fixed_cp1252("value", 8),
        "sized_cp1252": yggdryl.sized_cp1252("value", 32),
        "list": yggdryl.list("value", item),
        "list_view": yggdryl.list_view("value", item),
        "fixed_size_list": yggdryl.fixed_size_list("value", item, 3),
        "large_list": yggdryl.large_list("value", item),
        "large_list_view": yggdryl.large_list_view("value", item),
        "struct": yggdryl.struct("value", [item]),
        "union": yggdryl.union("value", [(3, item)], "dense"),
        "dictionary": yggdryl.dictionary("value", "int16", "utf8"),
        "decimal32": yggdryl.decimal32("value", 9, 2),
        "decimal64": yggdryl.decimal64("value", 18, 2),
        "decimal128": yggdryl.decimal128("value", 38, 2),
        "decimal256": yggdryl.decimal256("value", 76, 2),
        "map": yggdryl.map("value", entries),
        "sorted_map": yggdryl.map("value", entries, keys_sorted=True),
        "run_end_encoded": yggdryl.run_end_encoded(
            "value", run_ends, values
        ),
        "variant": yggdryl.variant("value"),
        "country": yggdryl.country("value"),
        "currency": yggdryl.currency("value"),
        "mic": yggdryl.mic("value"),
        "cfi": yggdryl.cfi("value"),
        "isin": yggdryl.isin("value"),
        "cusip": yggdryl.cusip("value"),
        "sedol": yggdryl.sedol("value"),
        "bloomberg": yggdryl.bloomberg("value"),
        "figi": yggdryl.figi("value"),
        "uuid": yggdryl.uuid("value"),
        "version": yggdryl.version("value"),
        "url": yggdryl.url("value"),
        "urn": yggdryl.urn("value"),
        "timezone": yggdryl.timezone("value"),
        "mimetype": yggdryl.mimetype("value"),
        "mediatype": yggdryl.mediatype("value"),
        "side": yggdryl.side("value"),
        "state": yggdryl.state("value"),
        "timeinforce": yggdryl.timeinforce("value"),
        "geometry": yggdryl.geometry("value"),
        "geography": yggdryl.geography("value", "OGC:CRS84", "vincenty"),
    }

    assert set(values_by_kind) == {
        value.dtype.id for value in values_by_kind.values()
    }
    # The factories cover every datatype Arrow has a layout for. `int128` and
    # `uint128` are the two identifiers `Scalar` stores and `DataType` cannot,
    # so no field builds them.
    unbuildable = {"int128", "uint128"}
    assert len(values_by_kind) == len(enums.DATA_TYPE_IDS) - len(unbuildable)
    assert set(values_by_kind) == set(enums.DATA_TYPE_IDS) - unbuildable
    assert all(type(value) is Field for value in values_by_kind.values())
    assert yggdryl.Int32Field is Field
    assert yggdryl.VersionField is Field
    assert yggdryl.UrlField is Field
    assert yggdryl.TimezoneField is Field
    assert yggdryl.MimeTypeField is Field
    assert yggdryl.MediaTypeField is Field
    assert yggdryl.TypedField is Field


def test_the_string_and_bytes_factories_take_the_whole_declaration() -> None:
    # One factory per family takes the layout, the charset, and the bound;
    # the leaf factories are that one with a leaf picked once. A maximum is
    # the sized leaf of the charset, so `max` beside a large layout is
    # refused by the name of the leaf that carries one.
    latin = yggdryl.string("name", layout="string", charset="cp1252", max=32)
    assert latin.dtype == DataType.string("sized_cp1252", bound=32)
    assert str(latin.dtype) == "sized_cp1252(32)"
    assert latin.dtype.string_parameters.max == 32
    assert latin.dtype == yggdryl.sized_cp1252("name", 32).dtype
    with pytest.raises(ValueError, match="sized_cp1252"):
        yggdryl.string("name", layout="large_string", charset="cp1252", max=32)
    fixed = yggdryl.string("code", layout="fixed_string", charset="us-ascii", fixed=4)
    assert fixed.dtype == DataType.fixed_ascii(4)
    assert yggdryl.string("code", layout="fixed_ascii", fixed=4).dtype == fixed.dtype
    assert yggdryl.string("text").dtype == DataType.utf8() == yggdryl.utf8("text").dtype
    assert yggdryl.sized_utf8("text", 32).dtype == DataType("utf8(32)")
    assert yggdryl.large_utf8_view("text").dtype == DataType("large_string_view")
    assert yggdryl.fixed_cp1252("text", 8).dtype == DataType("fixed_string(windows-1252,8)")
    assert yggdryl.string("text", nullable=False, metadata={"k": "v"}).metadata["k"] == "v"
    assert yggdryl.StringField is Field

    bounded = yggdryl.bytes("blob", max=16)
    assert bounded.dtype == DataType.bytes(bound=16) == DataType("binary(16)")
    assert bounded.dtype.bytes_parameters.max == 16
    digest = yggdryl.bytes("digest", layout="fixed_binary", fixed=16)
    assert digest.dtype == DataType.fixed_size_binary(16)
    assert digest.dtype == yggdryl.fixed_size_binary("digest", 16).dtype
    assert yggdryl.bytes("blob").dtype == DataType.binary() == yggdryl.binary("blob").dtype
    assert yggdryl.bytes("blob", layout="binary_view").dtype == DataType.binary_view()
    assert yggdryl.BytesField is Field


def test_nested_factories_preserve_exact_child_field_state() -> None:
    item = yggdryl.dictionary(
        "item",
        "int16",
        "utf8",
        nullable=False,
        metadata={"logical": "status"},
    )
    item.set_dictionary_options(42, True)
    projected_item = item.into_arrow()

    values = yggdryl.list("values", item, metadata={"owner": "events"})
    child = values.dtype[0]

    assert child.equals(item)
    assert child.dictionary_id == 42
    assert child.dictionary_is_ordered is True
    assert child.metadata["logical"] == "status"
    assert child.into_arrow().equals(projected_item, check_metadata=True)
    assert values.into_arrow().metadata == {b"owner": b"events"}


def test_dense_union_factory_is_a_typed_union_alias_with_native_ids() -> None:
    members = (
        yggdryl.int64("integer", nullable=False, metadata={"branch": "number"}),
        yggdryl.utf8("text", nullable=False),
    )

    value: yggdryl.DenseUnionField = yggdryl.dense_union(
        "payload",
        (member for member in members),
        nullable=False,
        metadata={"logical": "variant"},
    )
    arrow = value.into_arrow()

    assert type(value) is Field
    assert yggdryl.DenseUnionField is Field
    assert value.dtype.id == "union"
    assert tuple(value.dtype) == members
    assert arrow.type.mode == "dense"
    assert tuple(arrow.type.type_codes) == (0, 1)
    assert arrow.metadata == {b"logical": b"variant"}


def test_typed_factory_parameters_use_native_validation() -> None:
    assert yggdryl.decimal("small", 38).dtype.id == "decimal128"
    assert yggdryl.decimal("wide", 39).dtype.id == "decimal256"
    assert yggdryl.ascii("note").dtype == DataType("ascii")
    assert yggdryl.fixed_ascii("iso", 2).dtype == DataType.fixed_ascii(2)
    assert yggdryl.fixed_ascii("ccy", 3).dtype == DataType.fixed_ascii(3)
    # A fixed width past the packed integer is still storage, so it builds.
    assert yggdryl.fixed_ascii("isin", 64, nullable=False).dtype.fixed_byte_width == 64
    assert yggdryl.fixed_utf8("name", 8).dtype == DataType.fixed_utf8(8)
    assert yggdryl.fixed_utf8("name", 8).dtype.string_parameters.fixed == 8
    assert yggdryl.currency("ccy", metadata={"code": "ISO 4217"}).metadata["code"] == (
        "ISO 4217"
    )
    assert yggdryl.time("coarse", "ms").dtype == DataType("time32(ms)")
    assert yggdryl.time("precise", "us").dtype == DataType("time64(us)")
    assert yggdryl.datetime64("event", "us", "Custom/Accepted").dtype.id == (
        "datetime64"
    )

    with pytest.raises(ValueError, match="temporal resolution"):
        yggdryl.time("clock", "day_time")
    with pytest.raises(ValueError, match="temporal resolution"):
        yggdryl.datetime64("event", "year_month")
    with pytest.raises(ValueError, match="interval layout"):
        yggdryl.interval("window", "us")
    with pytest.raises(ValueError, match="precision"):
        yggdryl.decimal32("amount", 10)
    with pytest.raises(ValueError, match="at least one byte"):
        yggdryl.fixed_ascii("narrow", 0)
    with pytest.raises(ValueError, match="at least one byte"):
        yggdryl.fixed_size_binary("narrow", 0)
    with pytest.raises(ValueError, match="fixed_utf8"):
        yggdryl.string("narrow", layout="fixed_string")
    with pytest.raises(ValueError, match="expected no charset on ascii"):
        yggdryl.string("narrow", layout="ascii", charset="utf-8")
    with pytest.raises(TypeError, match="not both"):
        yggdryl.string("narrow", fixed=4, max=8)
    with pytest.raises(TypeError, match="not both"):
        yggdryl.bytes("narrow", fixed=4, max=8)
    with pytest.raises(ValueError, match="run_ends"):
        yggdryl.run_end_encoded(
            "encoded",
            yggdryl.int16("run_ends", nullable=True),
            yggdryl.utf8("values"),
        )


def test_field_and_datatype_equality_can_ignore_recursive_metadata() -> None:
    left_child = yggdryl.int32(
        "id", nullable=False, metadata={"source": "left"}
    )
    right_child = yggdryl.int32(
        "id", nullable=False, metadata={"source": "right"}
    )
    left = yggdryl.struct("row", [left_child], metadata={"root": "left"})
    right = yggdryl.struct("row", [right_child], metadata={"root": "right"})

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
    mapping = yggdryl.map_of(
        "labels", "utf8", "int32", keys_sorted=True, nullable=False
    )
    arrow = mapping.into_arrow()

    assert arrow.type.equals(pa.map_(pa.string(), pa.int32(), keys_sorted=True))
    assert arrow.nullable is False


def test_dictionary_and_map_of_infer_python_and_pyarrow_type_inputs() -> None:
    dictionary = yggdryl.dictionary("status", int, str, nullable=False)
    mapping = yggdryl.map_of("labels", str, pa.int16(), nullable=False)

    assert str(dictionary.dtype) == "dictionary(int64,utf8)"
    assert mapping.dtype.id == "map"
    entries = mapping.dtype[0].dtype
    assert [field.dtype.id for field in entries] == ["utf8", "int16"]
