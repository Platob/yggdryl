"""Tests for the struct surface (RecordScalar / StructType / StructField) in the
Python binding."""

import dataclasses

import pytest

from yggdryl import dtype, field, scalar


def test_record_builds_from_a_dict_of_native_values():
    row = scalar.RecordScalar({"x": 1, "blob": b"hi", "scores": [1, 2, 3], "gap": None})
    assert row.is_null() is False
    assert row.field_names() == ["x", "blob", "scores", "gap"]
    assert row.data_type().name() == "struct"
    assert row.data_type().child_count() == 4

    # get reads one child's native value; an unknown name is None.
    assert row.get("x") == 1
    assert row.get("blob") == b"hi"
    assert row.get("scores") == [1, 2, 3]
    assert row.get("gap") is None
    assert row.get("missing") is None

    # to_pydict copies the whole row out as native values.
    assert row.to_pydict() == {"x": 1, "blob": b"hi", "scores": [1, 2, 3], "gap": None}


def test_record_reads_a_float_field():
    # A Python float child infers a float64 field, read back as a Python float.
    row = scalar.RecordScalar({"x": 1, "weight": 1.5})
    assert row.field_names() == ["x", "weight"]
    assert row.data_type().field_names() == ["x", "weight"]
    assert row.get("weight") == 1.5
    assert row.to_pydict() == {"x": 1, "weight": 1.5}
    assert row.to_pyvalue().weight == 1.5


def test_record_reads_a_string_field():
    # A Python str child infers a utf8 field, read back as a Python str.
    row = scalar.RecordScalar({"id": 7, "name": "Ada"})
    assert row.field_names() == ["id", "name"]
    assert row.data_type().field_names() == ["id", "name"]
    assert row.get("name") == "Ada"
    assert row.to_pydict() == {"id": 7, "name": "Ada"}
    assert row.to_pyvalue().name == "Ada"


def test_record_to_pyvalue_is_a_singleton_dataclass_instance():
    row = scalar.RecordScalar({"x": 1, "y": 2})
    value = row.to_pyvalue()
    assert dataclasses.is_dataclass(value)
    assert value.x == 1
    assert value.y == 2

    # One auto-generated frozen class per schema: two records of the same field
    # names share it, a different schema gets its own.
    twin = scalar.RecordScalar({"x": 8, "y": 9}).to_pyvalue()
    assert type(value) is type(twin)
    other = scalar.RecordScalar({"x": 1, "z": 2}).to_pyvalue()
    assert type(value) is not type(other)
    with pytest.raises(dataclasses.FrozenInstanceError):
        value.x = 5


def test_record_nests():
    row = scalar.RecordScalar({"point": {"x": 1, "y": 2}, "id": 7})
    assert row.data_type().field_names() == ["point", "id"]
    nested = row.get("point")
    assert dataclasses.is_dataclass(nested)
    assert (nested.x, nested.y) == (1, 2)
    assert row.to_pyvalue().point == nested


def test_the_null_record_holds_no_row():
    struct = dtype.StructType({"x": 1})
    missing = scalar.RecordScalar.null(struct)
    assert missing.is_null() is True
    assert missing.field_names() == ["x"]
    assert missing.get("x") is None
    assert missing.to_pydict() is None
    assert missing.to_pyvalue() is None


def test_struct_type_resolves_example_values_and_dtype_instances():
    struct = dtype.StructType({
        "x": 1,
        "blob": dtype.BinaryType(),
        "scores": dtype.Int64SerieType(),
        "point": dtype.StructType({"y": 2}),
    })
    assert struct.name() == "struct"
    assert struct.arrow_format() == "+s"
    assert struct.byte_width() is None
    assert struct.bit_width() is None
    assert struct.child_count() == 4
    assert struct.field_names() == ["x", "blob", "scores", "point"]

    # A value the inference has no type for names the fix (a str is now a utf8
    # child, so bool remains a still-unsupported case).
    with pytest.raises(ValueError):
        dtype.StructType({"x": True})
    with pytest.raises(ValueError, match="str field name"):
        dtype.StructType({1: 2})


def test_struct_field_pairs_a_name_with_the_struct_type():
    struct = dtype.StructType({"x": 1})
    column = field.StructField("row", struct, False)
    assert column.name() == "row"
    assert column.data_type().name() == "struct"
    assert column.data_type().field_names() == ["x"]
    assert column.is_nullable() is False
    assert field.StructField("row", struct).is_nullable() is True  # by default
